use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};

use chrono::{NaiveDate, Utc};
use paddock_domain::{
    BetCombination, HorseProbability, PortfolioBet, PortfolioConfig, Race, RaceId, Surface,
    TrackCondition, build_portfolio,
};
use paddock_use_case::{PredictBetRecord, PredictSessionRecord};

use crate::setup::App;

/// 本番予想で使う市場オッズ(単勝)ブレンドのモデル重み α（#72）。`None` はモデルのみ。
/// backtest（2025-01〜2026-06, 4891R）の α スイープで Brier/LogLoss が α=0.2 で最良（ADR 0034）。
/// 市場オッズが無いレースは自動でモデルのみにフォールバックする。
/// 詳細は docs/specifications/probability-estimation.md。
const MARKET_BLEND_ALPHA: Option<f64> = Some(0.2);

/// 1 日分のレースを順番に処理する対話セッション。
///
/// 新規開始時は `budget` 必須でセッションを作成し、レース確定ごとに DB へ保存する。
/// `resume` が true なら保存済みセッションの残高から再開し、処理済みレースをスキップする。
pub async fn run_session(
    app: &App,
    date: NaiveDate,
    budget: Option<u64>,
    race_budget: u64,
    resume: bool,
) -> anyhow::Result<()> {
    let races = app.interactor.races_by_date(date).await?;
    if races.is_empty() {
        println!("この日の開催はありません: {}", date.format("%Y-%m-%d"));
        return Ok(());
    }

    let existing = app.interactor.find_predict_session(date).await?;
    let date_str = date.format("%Y-%m-%d").to_string();

    let (mut session, processed): (PredictSessionRecord, HashSet<String>) = if resume {
        let Some(session) = existing else {
            anyhow::bail!(
                "{date_str} のセッションがありません。新規開始は --resume なしで実行してください。"
            );
        };
        if session.completed {
            println!("{date_str} のセッションは完了済みです。集計は --summary を使ってください。");
            return Ok(());
        }
        if budget.is_some() {
            println!("注意: --resume では --budget は無視され、保存済み予算を使います。");
        }
        let bets = app.interactor.find_predict_bets(date).await?;
        let processed: HashSet<String> =
            bets.iter().map(|b| b.race_id.value().to_string()).collect();
        println!(
            "=== {date_str} 再開 — 残高 ¥{} / 処理済み {} レース ===",
            session.balance,
            processed.len()
        );
        (session, processed)
    } else {
        let Some(budget) = budget else {
            anyhow::bail!("新規セッションには --budget が必要です（例: --budget 10000）。");
        };
        // budget>0・同一開催日の二重作成ガード・残高初期化・開始時点のヘッダ保存（全レースを
        // スキップしても再開できるよう）は use-case の create_predict_session に集約済み（#164）。
        // 不変条件違反の Conflict/InvalidArgument は CLI 向けの案内文へ翻訳する（判定は use-case が担う）。
        let session = match app.interactor.create_predict_session(date, budget).await {
            Ok(session) => session,
            Err(paddock_use_case::Error::Conflict(_)) => anyhow::bail!(
                "{date_str} のセッションは既に存在します。続きは --resume、集計は --summary を使ってください。"
            ),
            Err(paddock_use_case::Error::InvalidArgument(_)) => {
                anyhow::bail!("予算は 1 以上を指定してください（例: --budget 10000）。")
            }
            Err(e) => return Err(e.into()),
        };
        println!("=== {date_str} 開催 — {} レース ===", races.len());
        println!("初期予算: ¥{budget}");
        (session, HashSet::new())
    };

    // 記録済みの馬場入力をロードし、resume 時のデフォルト提示に使う（新規セッションでは空）。
    // 同一セッション内では直前レースの入力を引き継いでデフォルト提示する（自動適用はしない）。
    let recorded: HashMap<String, Option<TrackCondition>> = app
        .interactor
        .find_predict_race_conditions(date)
        .await?
        .into_iter()
        .map(|r| (r.race_id.value().to_string(), r.track_condition))
        .collect();
    let mut last_input: Option<TrackCondition> = None;

    for race in &races {
        if processed.contains(race.race_id.value()) {
            continue;
        }
        run_race(
            app,
            race,
            &mut session,
            race_budget,
            &recorded,
            &mut last_input,
        )
        .await?;
    }

    session.completed = true;
    session.updated_at = Utc::now();
    app.interactor.save_predict_session(&session).await?;

    println!();
    println!("=== {date_str} 終了 ===");
    print_totals(&session);
    Ok(())
}

async fn run_race(
    app: &App,
    race: &Race,
    session: &mut PredictSessionRecord,
    race_budget: u64,
    recorded: &HashMap<String, Option<TrackCondition>>,
    last_input: &mut Option<TrackCondition>,
) -> anyhow::Result<()> {
    println!();
    println!(
        "--- レース {}: {} {} {}m ---",
        race.race_num,
        race.venue.as_jp(),
        surface_jp(race.surface),
        race.distance
    );
    println!("残高: ¥{}", session.balance);

    // 当日の馬場状態（#73）。未確定レースの race.track_condition は構造的に None
    //（races へ入るのは成績取り込み後）のため、レース毎に対話入力で受け取る。
    // デフォルトは「このセッションで記録済みの値（resume）→ 直前レースの入力 →
    // races の確定値」の優先順で決め、空入力で採用する（#80）。
    let default = resolve_track_condition_default(
        recorded.get(race.race_id.value()).copied(),
        *last_input,
        race.track_condition,
    );
    let track_condition = read_track_condition(&mut io::stdin().lock(), default)?;
    // 入力値は買い目の有無に依存せず記録し、「どの馬場前提で予想したか」を再現可能にする（#80）。
    // ただし resume 等で記録済みと同値なら、updated_at の無駄な更新（監査ノイズ）と
    // 冗長な書き込みを避けて保存を省く。`recorded` は run_session 冒頭でロードした不変の
    // スナップショットで、処理済みレースの再訪は呼び出し側の `processed` ガードで排除される。
    if recorded.get(race.race_id.value()).copied() != Some(track_condition) {
        app.interactor
            .save_predict_race_condition(session.date, &race.race_id, track_condition)
            .await?;
    }
    // 保存成功後に直前入力を更新する（保存失敗時は `?` で中断し、更新しない）。
    *last_input = track_condition;

    // 出馬表未登録（NotFound）はそのレースのみスキップ。
    // DB 障害等（Internal）はセッション継続不能なため伝播して中断する。
    let probs = match app
        .interactor
        .predict_race(&race.race_id, MARKET_BLEND_ALPHA, track_condition)
        .await
    {
        Ok(p) => p,
        Err(paddock_use_case::Error::NotFound(msg)) => {
            println!("出馬表が見つかりません（{msg}）。スキップします。");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    println!();
    print_probs(&probs);

    // オッズ未取得（None）はスキップのみ受付（select_bets は呼ばない）。
    // OddsInteractor が都度ライブスクレイプし、取得失敗・未公開は None に畳む。
    let Some(odds) = app.odds.race_odds(&race.race_id).await? else {
        println!();
        println!("オッズ未取得 — このレースはスキップします");
        let _ = read_line(&mut io::stdin().lock(), "Enter で次のレースへ > ")?;
        return Ok(());
    };

    // 軸流しポートフォリオ（馬連＋ワイド＋三連複）を予算内・100 円単位で生成する。
    // 上限は per-race 予算と残高の小さい方。配分・相手頭数は PortfolioConfig 既定（#122）。
    let race_cap = race_budget.min(session.balance);
    let portfolio = build_portfolio(&probs, &odds, race_cap, &PortfolioConfig::default());
    let suggested: Vec<u64> = portfolio.bets.iter().map(|b| b.stake).collect();

    println!();
    println!("【買い目推奨（軸流し, 予算¥{race_cap}/R）】");
    match portfolio.axis {
        Some(axis) => {
            let partners = portfolio
                .partners
                .iter()
                .map(|h| h.value().to_string())
                .collect::<Vec<_>>()
                .join(",");
            println!("  軸 {} → 相手 {}", axis.value(), partners);
        }
        None => println!("  確率推定が空のため買い目なし"),
    }
    if portfolio.bets.is_empty() {
        println!("  予算内で組める買い目なし");
    }
    for bet in &portfolio.bets {
        let odds = match bet.odds {
            Some(o) => format!("オッズ{o:.1}"),
            None => "オッズ未取得".to_string(),
        };
        println!(
            "  {} ¥{} {} EV={:.2}",
            format_combination(&bet.combination),
            bet.stake,
            odds,
            bet.ev,
        );
    }
    if let Some(ev) = &portfolio.ev {
        // 期待回収率・的中率はオッズ取得済みの脚についての値（未取得脚は払戻を見積もれず除外）。
        let unpriced = portfolio.bets.iter().filter(|b| b.odds.is_none()).count();
        // 回収率・的中率はオッズ取得済の脚のみで算出する一方、賭け計は未取得脚も含む全脚の合計
        // （基準が異なる）。未取得脚があるときはその非対称を明示する。
        let note = if unpriced > 0 {
            format!(
                "（回収率・的中率はオッズ取得済の脚基準、賭け計は未取得 {unpriced} 点を含む全脚）"
            )
        } else {
            String::new()
        };
        println!(
            "  ポートフォリオ期待回収率 {:.1}% / 的中率 {:.1}% / 賭け計 ¥{}{}",
            ev.roi * 100.0,
            ev.hit_prob * 100.0,
            portfolio.total_stake,
            note,
        );
    }

    println!();
    let bet_amounts: Vec<u64> = match read_choice(&mut io::stdin().lock())? {
        's' => return Ok(()),
        'y' => suggested.clone(),
        'e' => read_edited_amounts(
            &mut io::stdin().lock(),
            &portfolio.bets,
            &suggested,
            session.balance,
        )?,
        _ => unreachable!("read_choice returns only y/e/s"),
    };

    let bet: u64 = bet_amounts.iter().sum();
    if bet == 0 {
        println!("賭けなし — 次のレースへ");
        return Ok(());
    }

    println!();
    println!(">>> レース後 — 買い目ごとに払戻を入力 <<<");
    // 賭け金 > 0 の買い目だけを対象に払戻を入力し、その場でレコード化する
    // （stake==0 の判定はこの 1 箇所に集約）。
    let mut bet_records = Vec::new();
    for (bet_item, &stake) in portfolio.bets.iter().zip(&bet_amounts) {
        if stake == 0 {
            continue;
        }
        let payout = read_u64(
            &mut io::stdin().lock(),
            &format!(
                "  {} 賭け¥{} の払戻 (なし: Enter) > ",
                format_combination(&bet_item.combination),
                stake
            ),
            true,
        )?;
        bet_records.push(make_bet_record(
            &race.race_id,
            &bet_item.combination,
            bet_item.ev,
            stake,
            payout,
        ));
    }
    let race_payout: u64 = bet_records.iter().map(|b| b.payout).sum();

    // 残高ガード（Σstake ≤ balance）・残高/累計計算・セッション更新＋買い目追記の 1 トランザクション
    // 保存・updated_at の時刻注入は use-case の record_race_outcome に集約済み（#164）。推奨は
    // race_cap=min(race_budget, balance)、編集は read_edited_amounts が balance 上限を強制するため、
    // ここに到達する bet は常に残高内・未記録だが、use-case が防御的に返す残高超過（InvalidArgument）・
    // 二重記録（Conflict）はセッション全体を中断せず当該レースをスキップして継続する（旧「残高超過
    // スキップ」挙動を踏襲）。成功時は DB 反映済みの更新後セッションで丸ごと置換し、残高表示に使う。
    *session = match app
        .interactor
        .record_race_outcome(session.date, &race.race_id, bet_records)
        .await
    {
        Ok(updated) => updated,
        Err(paddock_use_case::Error::InvalidArgument(_)) => {
            println!("賭け金合計が残高を超えるため、このレースをスキップします。");
            return Ok(());
        }
        Err(paddock_use_case::Error::Conflict(_)) => {
            println!("このレースは既に記録済みのため、スキップします。");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    let pnl = race_payout as i128 - bet as i128;
    println!(
        "  賭け金: ¥{}  払戻: ¥{}  ({})",
        bet,
        race_payout,
        format_signed(pnl)
    );
    println!("残高: ¥{}", session.balance);

    Ok(())
}

/// 同日セッションの収支サマリと買い目明細を表示する（--summary、読み取り専用）。
pub async fn print_session_summary(app: &App, date: NaiveDate) -> anyhow::Result<()> {
    let date_str = date.format("%Y-%m-%d").to_string();
    let Some(session) = app.interactor.find_predict_session(date).await? else {
        println!("{date_str} のセッションはありません。");
        return Ok(());
    };

    println!(
        "=== {date_str} セッション収支{} ===",
        if session.completed {
            ""
        } else {
            "（未完了）"
        }
    );
    println!("開始予算: ¥{}", session.budget);
    println!("現在残高: ¥{}", session.balance);
    print_totals(&session);

    let bets = app.interactor.find_predict_bets(date).await?;
    if !bets.is_empty() {
        println!();
        println!("【買い目明細】");
        println!(
            "{:<22} {:<10} {:<10} {:>8} {:>8} {:>6}",
            "レース", "馬券種", "組合せ", "賭け金", "払戻", "EV"
        );
        for b in &bets {
            println!(
                "{:<22} {:<10} {:<10} {:>7}円 {:>7}円 {:>6.2}",
                b.race_id.value(),
                b.bet_type,
                b.combination,
                b.stake,
                b.payout,
                b.ev,
            );
        }
    }
    Ok(())
}

/// 確定払戻でセッションを事後精算する（--settle、#40）。netkeiba の確定払戻で購入済み
/// 買い目の payout を自動セットし、収支・回収率を更新する（冪等。未確定はスキップ）。
pub async fn run_settle(app: &App, date: NaiveDate) -> anyhow::Result<()> {
    let date_str = date.format("%Y-%m-%d").to_string();
    println!("=== {date_str} 自動精算 ===");
    let report = match app.settle.settle_session(date).await {
        Ok(r) => r,
        Err(paddock_use_case::Error::NotFound(msg)) => {
            println!("{msg}。先に予想セッションを実行してください。");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    println!("確定レース: {}", report.settled_races);
    if report.pending_races > 0 {
        println!(
            "未確定レース: {}（payout 据え置き。確定後に再実行してください）",
            report.pending_races
        );
    }
    if report.voided_races > 0 {
        println!(
            "全額返還レース: {}（開催中止・全馬取消で全買い目に stake 返戻）",
            report.voided_races
        );
    }
    if report.refunded_bets > 0 {
        println!(
            "返還: {}件（取消/除外を含む組番に stake 返戻）",
            report.refunded_bets
        );
    }
    println!("総賭け金: ¥{}", report.total_bet);
    println!("総払戻:   ¥{}", report.total_payout);
    println!("最終残高: ¥{}", report.balance);
    let pnl = report.total_payout as i128 - report.total_bet as i128;
    println!("P&L:      {}", format_signed(pnl));
    if let Some(roi) = report.roi {
        println!("回収率:   {roi:.1}%");
    }

    // 明細（更新後の payout）を表示する。
    print_session_summary(app, date).await
}

/// 1 件の買い目を DB 保存用レコードに変換する純関数。馬券種ラベル・組み合わせコード・
/// 各フィールド（残高・回収率に直結）のマッピングを対話 I/O から切り離して単体テストできる。
fn make_bet_record(
    race_id: &RaceId,
    combination: &BetCombination,
    ev: f64,
    stake: u64,
    payout: u64,
) -> PredictBetRecord {
    PredictBetRecord {
        race_id: race_id.clone(),
        bet_type: combination.type_label().to_string(),
        combination: combination.combination_code(),
        stake,
        payout,
        ev,
    }
}

fn read_edited_amounts<R: BufRead>(
    reader: &mut R,
    bets: &[PortfolioBet],
    suggested: &[u64],
    budget: u64,
) -> anyhow::Result<Vec<u64>> {
    loop {
        let mut amounts = Vec::with_capacity(bets.len());
        for (bet, sug) in bets.iter().zip(suggested) {
            let a = read_u64(
                reader,
                &format!(
                    "  {} 推奨¥{} 入力額 > ",
                    format_combination(&bet.combination),
                    sug
                ),
                false,
            )?;
            amounts.push(a);
        }
        let total: u64 = amounts.iter().sum();
        if total > budget {
            println!("合計 ¥{total} が残高 ¥{budget} を超えています。入力し直してください。");
            continue;
        }
        return Ok(amounts);
    }
}

fn print_totals(session: &PredictSessionRecord) {
    println!("総賭け金: ¥{}", session.total_bet);
    println!("総払戻:   ¥{}", session.total_payout);
    println!("最終残高: ¥{}", session.balance);
    let pnl = session.total_payout as i128 - session.total_bet as i128;
    println!("P&L:      {}", format_signed(pnl));
    if session.total_bet > 0 {
        let roi = session.total_payout as f64 / session.total_bet as f64 * 100.0;
        println!("回収率:   {roi:.1}%");
    }
}

fn print_probs(probs: &[HorseProbability]) {
    println!(
        "{:<4} {:<16} {:>8} {:>8} {:>8}",
        "馬番", "馬名", "勝率", "連対率", "複勝率"
    );
    for p in probs {
        println!(
            "{:>4} {:<16} {:>7.1}% {:>7.1}% {:>7.1}%",
            p.horse_num.value(),
            p.horse_name.value(),
            p.win_prob * 100.0,
            p.place_prob * 100.0,
            p.show_prob * 100.0,
        );
    }
}

fn format_combination(c: &BetCombination) -> String {
    match c {
        BetCombination::Win(h) => format!("単勝 {}", h.value()),
        BetCombination::Place(h) => format!("複勝 {}", h.value()),
        BetCombination::Quinella(p) => {
            let (a, b) = p.as_tuple();
            format!("馬連 {}-{}", a.value(), b.value())
        }
        BetCombination::Wide(p) => {
            let (a, b) = p.as_tuple();
            format!("ワイド {}-{}", a.value(), b.value())
        }
        BetCombination::Exacta(p) => {
            let (a, b) = p.as_tuple();
            format!("馬単 {}→{}", a.value(), b.value())
        }
        BetCombination::Trio(t) => {
            let (a, b, c) = t.as_tuple();
            format!("三連複 {}-{}-{}", a.value(), b.value(), c.value())
        }
        BetCombination::Trifecta(t) => {
            let (a, b, c) = t.as_tuple();
            format!("三連単 {}→{}→{}", a.value(), b.value(), c.value())
        }
    }
}

fn surface_jp(s: Surface) -> &'static str {
    match s {
        Surface::Turf => "芝",
        Surface::Dirt => "ダート",
    }
}

fn format_signed(v: i128) -> String {
    if v >= 0 {
        format!("+¥{v}")
    } else {
        format!("-¥{}", -v)
    }
}

/// 1 行読み取る。EOF（読み取り 0 バイト）は `None` を返し、呼び出し側が安全側へ畳めるようにする。
/// 旧実装は EOF でも空文字 `Ok("")` を返していたため、`read_choice` のような再プロンプトループが
/// EOF 後にブロックせず無限に回り続けて出力が暴走した（#179）。
fn read_line<R: BufRead>(reader: &mut R, prompt: &str) -> io::Result<Option<String>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut buf = String::new();
    if reader.read_line(&mut buf)? == 0 {
        return Ok(None);
    }
    Ok(Some(buf.trim().to_string()))
}

/// `y` / `e` / `s` のいずれかを読み取る（不正入力は再プロンプト）。
/// EOF はスキップ（`s`）扱いにして無限ループを断つ（#179）。
fn read_choice<R: BufRead>(reader: &mut R) -> anyhow::Result<char> {
    loop {
        match read_line(
            reader,
            "購入方法を選んでください [y=推奨通り / e=編集 / s=スキップ] > ",
        )? {
            None => return Ok('s'),
            Some(s) => match s.as_str() {
                "y" | "Y" => return Ok('y'),
                "e" | "E" => return Ok('e'),
                "s" | "S" => return Ok('s'),
                _ => println!("y / e / s のいずれかを入力してください。"),
            },
        }
    }
}

/// レース冒頭の馬場入力デフォルトを決める純関数（#80）。優先順は
/// 「このセッションで記録済みの値 → 同一セッション内の直前レース入力 → races の確定値」。
///
/// `recorded` はセッション記録テーブルの引き当て結果。`Some(stored)` はこのレースを既に
/// 入力済み（`stored` が `None` でも「不明として入力済み」を意味する）で、resume 時は
/// この値を最優先する。未記録（`None`）のときのみ直前入力 `last_input`、無ければ確定値
/// `official` にフォールバックする。
fn resolve_track_condition_default(
    recorded: Option<Option<TrackCondition>>,
    last_input: Option<TrackCondition>,
    official: Option<TrackCondition>,
) -> Option<TrackCondition> {
    match recorded {
        Some(stored) => stored,
        None => last_input.or(official),
    }
}

/// 当日の馬場状態を読み取る（#73）。空入力は `default`（DB 値があればそれ、無ければ None=
/// 馬場項なし）を採用し、`-` は不明（None）を明示する。不正入力は再プロンプト。
/// 「稍」「不」の略記も受け付ける。
/// EOF は空入力と同じくデフォルト採用で抜ける（#179）。
fn read_track_condition<R: BufRead>(
    reader: &mut R,
    default: Option<TrackCondition>,
) -> anyhow::Result<Option<TrackCondition>> {
    let prompt = match default {
        Some(tc) => format!("馬場状態 [良/稍重/重/不良, 空={tc}, -=不明] > "),
        None => "馬場状態 [良/稍重/重/不良, 空=不明] > ".to_string(),
    };
    loop {
        let Some(s) = read_line(reader, &prompt)? else {
            return Ok(default);
        };
        if s.is_empty() {
            return Ok(default);
        }
        // IME 入力を考慮して全角ハイフン・長音も不明扱いで受ける。
        if matches!(s.as_str(), "-" | "－" | "ー") {
            return Ok(None);
        }
        match TrackCondition::try_from(s.as_str()) {
            Ok(tc) => return Ok(Some(tc)),
            Err(_) => println!(
                "良 / 稍重 / 重 / 不良（稍・不 の略記可）、空、または - を入力してください。"
            ),
        }
    }
}

/// 非負整数を読み取る。`allow_empty_as_zero` が true なら空入力を 0 とみなす。
/// EOF はこれ以上入力が無いので 0（賭けなし）扱いで抜ける（#179）。
fn read_u64<R: BufRead>(
    reader: &mut R,
    prompt: &str,
    allow_empty_as_zero: bool,
) -> anyhow::Result<u64> {
    loop {
        let Some(s) = read_line(reader, prompt)? else {
            return Ok(0);
        };
        if s.is_empty() && allow_empty_as_zero {
            return Ok(0);
        }
        match s.parse::<u64>() {
            Ok(v) => return Ok(v),
            Err(_) => println!("数値を入力してください。"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        make_bet_record, read_choice, read_edited_amounts, read_track_condition, read_u64,
        resolve_track_condition_default,
    };
    use paddock_domain::horse_result::HorseNum;
    use paddock_domain::{BetCombination, PortfolioBet, RaceId, TrackCondition};
    use std::io::Cursor;

    fn horse(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    #[test]
    fn make_bet_record_maps_fields() {
        let race_id = RaceId::try_from("2026-3-nakayama-8-1R").unwrap();

        let win = make_bet_record(&race_id, &BetCombination::Win(horse(3)), 1.5, 1000, 0);
        assert_eq!(win.bet_type, "win");
        assert_eq!(win.combination, "3");
        assert_eq!(win.stake, 1000);
        assert_eq!(win.payout, 0);
        assert!((win.ev - 1.5).abs() < 1e-10);
        assert_eq!(win.race_id.value(), "2026-3-nakayama-8-1R");

        let quinella =
            BetCombination::Quinella(paddock_domain::Pair::try_from((horse(1), horse(5))).unwrap());
        let q = make_bet_record(&race_id, &quinella, 1.2, 500, 2500);
        assert_eq!(q.bet_type, "quinella");
        assert_eq!(q.combination, "1-5");
        assert_eq!(q.stake, 500);
        assert_eq!(q.payout, 2500);
    }

    #[test]
    fn track_default_prefers_recorded_value_on_resume() {
        // 記録済み（resume）の値は直前入力・確定値より優先される。
        let d = resolve_track_condition_default(
            Some(Some(TrackCondition::Good)),
            Some(TrackCondition::Firm),
            Some(TrackCondition::Soft),
        );
        assert_eq!(d, Some(TrackCondition::Good));
    }

    #[test]
    fn track_default_recorded_unknown_stays_none() {
        // 「不明として記録済み」(Some(None)) は None を維持し、フォールバックしない。
        let d = resolve_track_condition_default(
            Some(None),
            Some(TrackCondition::Firm),
            Some(TrackCondition::Soft),
        );
        assert_eq!(d, None);
    }

    #[test]
    fn track_default_falls_back_to_last_input_when_unrecorded() {
        // 未記録なら同一セッション内の直前入力を確定値より優先してデフォルト提示する。
        let d = resolve_track_condition_default(
            None,
            Some(TrackCondition::Yielding),
            Some(TrackCondition::Firm),
        );
        assert_eq!(d, Some(TrackCondition::Yielding));
    }

    #[test]
    fn track_default_falls_back_to_official_when_no_input() {
        // 未記録かつ直前入力も無ければ races の確定値を使う。
        let d = resolve_track_condition_default(None, None, Some(TrackCondition::Firm));
        assert_eq!(d, Some(TrackCondition::Firm));
    }

    #[test]
    fn track_default_all_none_is_none() {
        let d = resolve_track_condition_default(None, None, None);
        assert_eq!(d, None);
    }

    // --- stdin reader の EOF 挙動（#179: EOF で無限ループしないこと）---

    #[test]
    fn read_choice_returns_skip_on_eof() {
        // 空入力（即 EOF）は無限ループせずスキップ(s)で抜ける。
        let mut r = Cursor::new(b"".to_vec());
        assert_eq!(read_choice(&mut r).unwrap(), 's');
    }

    #[test]
    fn read_choice_reprompts_then_skips_on_eof() {
        // 不正入力を1回挟んでも、後続が EOF ならスキップで確定する（再プロンプトが無限化しない）。
        let mut r = Cursor::new(b"x\n".to_vec());
        assert_eq!(read_choice(&mut r).unwrap(), 's');
    }

    #[test]
    fn read_choice_parses_valid_input() {
        let mut r = Cursor::new(b"y\n".to_vec());
        assert_eq!(read_choice(&mut r).unwrap(), 'y');
    }

    #[test]
    fn read_track_condition_eof_takes_default() {
        // EOF は空入力と同じくデフォルト採用で抜ける。
        let mut r = Cursor::new(b"".to_vec());
        let d = read_track_condition(&mut r, Some(TrackCondition::Good)).unwrap();
        assert_eq!(d, Some(TrackCondition::Good));
    }

    #[test]
    fn read_u64_eof_is_zero() {
        // EOF は「これ以上入力なし」= 0（賭けなし）で抜ける。
        let mut r = Cursor::new(b"".to_vec());
        assert_eq!(read_u64(&mut r, "> ", false).unwrap(), 0);
    }

    #[test]
    fn read_track_condition_reprompts_then_eof_takes_default() {
        // 不正入力 → 後続 EOF でも再プロンプトループが無限化せず default で抜ける。
        let mut r = Cursor::new(b"xxx\n".to_vec());
        let d = read_track_condition(&mut r, Some(TrackCondition::Firm)).unwrap();
        assert_eq!(d, Some(TrackCondition::Firm));
    }

    #[test]
    fn read_u64_reprompts_then_eof_is_zero() {
        // 数値でない入力 → 後続 EOF でも無限化せず 0 で抜ける。
        let mut r = Cursor::new(b"abc\n".to_vec());
        assert_eq!(read_u64(&mut r, "> ", false).unwrap(), 0);
    }

    #[test]
    fn read_edited_amounts_eof_returns_zeros_without_looping() {
        // 'e'（編集）経路で途中 EOF になっても、全脚 0（賭けなし）を返して
        // 外側の再プロンプトループ（total>budget）が無限化しない（#179）。
        let bets = vec![
            PortfolioBet {
                combination: BetCombination::Win(horse(1)),
                stake: 500,
                odds: None,
                ev: 0.0,
            },
            PortfolioBet {
                combination: BetCombination::Win(horse(2)),
                stake: 300,
                odds: None,
                ev: 0.0,
            },
        ];
        let suggested = vec![500, 300];
        let mut r = Cursor::new(b"".to_vec());
        let amounts = read_edited_amounts(&mut r, &bets, &suggested, 10000).unwrap();
        assert_eq!(amounts, vec![0, 0]);
    }

    #[test]
    fn read_edited_amounts_overbudget_then_eof_terminates() {
        // 1周目で予算超過 → 外側の再プロンプトループに入り、2周目が EOF でも
        // 全0で確定して無限ループしない（コメントが謳う外側ループの終端を直接検証）。
        let bets = vec![PortfolioBet {
            combination: BetCombination::Win(horse(1)),
            stake: 100,
            odds: None,
            ev: 0.0,
        }];
        let suggested = vec![100];
        // 1周目: 20000 を入力（budget 10000 超過）→ 再プロンプト。2周目: EOF → 0。
        let mut r = Cursor::new(b"20000\n".to_vec());
        let amounts = read_edited_amounts(&mut r, &bets, &suggested, 10000).unwrap();
        assert_eq!(amounts, vec![0]);
    }
}
