use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

use chrono::{NaiveDate, Utc};
use paddock_domain::{
    BetCombination, BettingConfig, BettingRecommendation, HorseProbability, Race, RaceId, Surface,
    TrackCondition, select_bets,
};
use paddock_use_case::{PredictBetRecord, PredictSessionRecord};

use crate::setup::App;

/// 本番予想で使う市場オッズ(単勝)ブレンドのモデル重み α（#72）。`None` はモデルのみ。
/// backtest（2026-03〜05, 144R）の α スイープで、的中率・回収率が最良かつ校正も
/// 市場のみに近い α=0.3（市場重み 0.7）を採用。市場オッズが無いレースは自動でモデルのみに
/// フォールバックする。詳細は docs/specifications/probability-estimation.md。
const MARKET_BLEND_ALPHA: Option<f64> = Some(0.3);

/// 1 日分のレースを順番に処理する対話セッション。
///
/// 新規開始時は `budget` 必須でセッションを作成し、レース確定ごとに DB へ保存する。
/// `resume` が true なら保存済みセッションの残高から再開し、処理済みレースをスキップする。
pub async fn run_session(
    app: &App,
    date: NaiveDate,
    budget: Option<u64>,
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
        if existing.is_some() {
            anyhow::bail!(
                "{date_str} のセッションは既に存在します。続きは --resume、集計は --summary を使ってください。"
            );
        }
        let Some(budget) = budget else {
            anyhow::bail!("新規セッションには --budget が必要です（例: --budget 10000）。");
        };
        let now = Utc::now();
        let session = PredictSessionRecord {
            date,
            budget,
            balance: budget,
            total_bet: 0,
            total_payout: 0,
            completed: false,
            created_at: now,
            updated_at: now,
        };
        // 全レースをスキップしても再開できるよう、開始時点でヘッダを保存する。
        app.interactor.save_predict_session(&session).await?;
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
        run_race(app, race, &mut session, &recorded, &mut last_input).await?;
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
    let track_condition = read_track_condition(default)?;
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
        let _ = read_line("Enter で次のレースへ > ")?;
        return Ok(());
    };

    let recs = select_bets(&probs, &odds, &BettingConfig::default());
    let kelly_fractions: Vec<f64> = recs.iter().map(|r| r.kelly_fraction).collect();
    let suggested = recommended_amounts(session.balance, &kelly_fractions);

    println!();
    println!("【買い目推奨】");
    if recs.is_empty() {
        println!("  EV 閾値を超える買い目なし");
    }
    for (rec, amt) in recs.iter().zip(&suggested) {
        println!(
            "  {} EV={:.2} Kelly={:.0}% 推奨額=¥{}",
            format_combination(&rec.combination),
            rec.ev,
            rec.kelly_fraction * 100.0,
            amt,
        );
    }

    println!();
    let bet_amounts: Vec<u64> = match read_choice()? {
        's' => return Ok(()),
        'y' => suggested.clone(),
        'e' => read_edited_amounts(&recs, &suggested, session.balance)?,
        _ => unreachable!("read_choice returns only y/e/s"),
    };

    let bet: u64 = bet_amounts.iter().sum();
    if bet == 0 {
        println!("賭けなし — 次のレースへ");
        return Ok(());
    }
    // 残高ガード（y の比例縮小・e の入力チェックで保証されるが二重防御）
    if bet > session.balance {
        println!(
            "賭け金合計 ¥{} が残高 ¥{} を超えるためスキップします",
            bet, session.balance
        );
        return Ok(());
    }

    // 賭け金を先に差し引き、買い目ごとに払戻を入力する（per-bet 記録）。
    session.balance -= bet;
    session.total_bet += bet;

    println!();
    println!(">>> レース後 — 買い目ごとに払戻を入力 <<<");
    // 賭け金 > 0 の買い目だけを対象に払戻を入力し、その場でレコード化する
    // （stake==0 の判定はこの 1 箇所に集約）。
    let mut bet_records = Vec::new();
    for (rec, &stake) in recs.iter().zip(&bet_amounts) {
        if stake == 0 {
            continue;
        }
        let payout = read_u64(
            &format!(
                "  {} 賭け¥{} の払戻 (なし: Enter) > ",
                format_combination(&rec.combination),
                stake
            ),
            true,
        )?;
        bet_records.push(make_bet_record(&race.race_id, rec, stake, payout));
    }
    let race_payout: u64 = bet_records.iter().map(|b| b.payout).sum();
    session.balance += race_payout;
    session.total_payout += race_payout;

    // セッション更新＋このレースの買い目を 1 トランザクションで保存する。
    session.updated_at = Utc::now();
    app.interactor
        .save_race_outcome(session, &race.race_id, &bet_records)
        .await?;

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
    rec: &BettingRecommendation,
    stake: u64,
    payout: u64,
) -> PredictBetRecord {
    PredictBetRecord {
        race_id: race_id.clone(),
        bet_type: rec.combination.type_label().to_string(),
        combination: rec.combination.combination_code(),
        stake,
        payout,
        ev: rec.ev,
    }
}

/// Kelly 比例縮小方式で各買い目の推奨額を算出する。
///
/// 丸め前の実数合計 `Σ raw_i` を分母に使うことで、`floor` の単調性により
/// `Σ 推奨額 ≤ budget` を厳密に保証する（設計書 predict-session.md 参照）。
fn recommended_amounts(budget: u64, kelly_fractions: &[f64]) -> Vec<u64> {
    if kelly_fractions.is_empty() {
        return Vec::new();
    }
    let budget_f = budget as f64;
    let raws: Vec<f64> = kelly_fractions.iter().map(|k| budget_f * k).collect();
    let sum: f64 = raws.iter().sum();
    let mut amounts: Vec<u64> = if sum <= budget_f {
        raws.iter().map(|r| r.floor() as u64).collect()
    } else {
        raws.iter()
            .map(|r| (r * budget_f / sum).floor() as u64)
            .collect()
    };

    // 浮動小数の丸め誤差に対する最終防御: 合計が budget を超える場合は
    // 末尾の買い目から削り、u64 として確実に budget 以内へ収める。
    let mut total: u64 = amounts.iter().sum();
    let mut i = amounts.len();
    while total > budget && i > 0 {
        i -= 1;
        let cut = (total - budget).min(amounts[i]);
        amounts[i] -= cut;
        total -= cut;
    }
    amounts
}

fn read_edited_amounts(
    recs: &[BettingRecommendation],
    suggested: &[u64],
    budget: u64,
) -> anyhow::Result<Vec<u64>> {
    loop {
        let mut amounts = Vec::with_capacity(recs.len());
        for (rec, sug) in recs.iter().zip(suggested) {
            let a = read_u64(
                &format!(
                    "  {} 推奨¥{} 入力額 > ",
                    format_combination(&rec.combination),
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

fn read_line(prompt: &str) -> io::Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

/// `y` / `e` / `s` のいずれかを読み取る（不正入力は再プロンプト）。
fn read_choice() -> anyhow::Result<char> {
    loop {
        let s = read_line("購入方法を選んでください [y=推奨通り / e=編集 / s=スキップ] > ")?;
        match s.as_str() {
            "y" | "Y" => return Ok('y'),
            "e" | "E" => return Ok('e'),
            "s" | "S" => return Ok('s'),
            _ => println!("y / e / s のいずれかを入力してください。"),
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
fn read_track_condition(default: Option<TrackCondition>) -> anyhow::Result<Option<TrackCondition>> {
    let prompt = match default {
        Some(tc) => format!("馬場状態 [良/稍重/重/不良, 空={tc}, -=不明] > "),
        None => "馬場状態 [良/稍重/重/不良, 空=不明] > ".to_string(),
    };
    loop {
        let s = read_line(&prompt)?;
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
fn read_u64(prompt: &str, allow_empty_as_zero: bool) -> anyhow::Result<u64> {
    loop {
        let s = read_line(prompt)?;
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
    use super::{make_bet_record, recommended_amounts, resolve_track_condition_default};
    use paddock_domain::horse_result::HorseNum;
    use paddock_domain::{BetCombination, BettingRecommendation, RaceId, TrackCondition};

    fn rec(combination: BetCombination, ev: f64) -> BettingRecommendation {
        BettingRecommendation {
            combination,
            probability: 0.0,
            odds: 0.0,
            ev,
            kelly_fraction: 0.0,
        }
    }

    fn horse(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    #[test]
    fn make_bet_record_maps_fields() {
        let race_id = RaceId::try_from("2026-3-nakayama-8-1R").unwrap();

        let win = make_bet_record(&race_id, &rec(BetCombination::Win(horse(3)), 1.5), 1000, 0);
        assert_eq!(win.bet_type, "win");
        assert_eq!(win.combination, "3");
        assert_eq!(win.stake, 1000);
        assert_eq!(win.payout, 0);
        assert!((win.ev - 1.5).abs() < 1e-10);
        assert_eq!(win.race_id.value(), "2026-3-nakayama-8-1R");

        let place = make_bet_record(
            &race_id,
            &rec(BetCombination::Place(horse(7)), 1.2),
            500,
            2500,
        );
        assert_eq!(place.bet_type, "place");
        assert_eq!(place.combination, "7");
        assert_eq!(place.stake, 500);
        assert_eq!(place.payout, 2500);
    }

    #[test]
    fn within_budget_keeps_floor() {
        // budget 10000, kelly 0.15/0.08/0.05 → 1500/800/500（合計 2800 ≤ 10000）
        let amounts = recommended_amounts(10000, &[0.15, 0.08, 0.05]);
        assert_eq!(amounts, vec![1500, 800, 500]);
    }

    #[test]
    fn four_quarter_kelly_exactly_fits() {
        // 0.25 × 4 = 1.0、raw 合計 = 10000 = budget（縮小不要）
        let amounts = recommended_amounts(10000, &[0.25, 0.25, 0.25, 0.25]);
        assert_eq!(amounts, vec![2500, 2500, 2500, 2500]);
        assert_eq!(amounts.iter().sum::<u64>(), 10000);
    }

    #[test]
    fn over_budget_scales_down_within_balance() {
        // 0.25 × 5 = 1.25、raw 合計 12500 > 10000 → 比例縮小で各 2000
        let amounts = recommended_amounts(10000, &[0.25, 0.25, 0.25, 0.25, 0.25]);
        let total: u64 = amounts.iter().sum();
        assert!(total <= 10000, "total {total} must be <= budget");
        assert_eq!(amounts, vec![2000, 2000, 2000, 2000, 2000]);
    }

    #[test]
    fn floor_residual_never_exceeds_budget() {
        // 丸め前合計を分母にすることで floor 残差でも budget を超えないこと
        let amounts = recommended_amounts(
            58,
            &[0.2203, 0.1163, 0.0605, 0.2041, 0.2055, 0.1673, 0.1646],
        );
        let total: u64 = amounts.iter().sum();
        assert!(total <= 58, "total {total} must be <= 58");
    }

    #[test]
    fn empty_returns_empty() {
        assert!(recommended_amounts(10000, &[]).is_empty());
    }

    #[test]
    fn extreme_budget_never_exceeds() {
        // 非現実的な巨大予算でも float 丸め誤差を最終クランプで吸収し
        // u64 として budget を超えないこと
        let budget = u64::MAX / 2;
        let amounts = recommended_amounts(budget, &[0.25, 0.25, 0.25, 0.25, 0.25]);
        let total: u128 = amounts.iter().map(|&a| a as u128).sum();
        assert!(total <= budget as u128, "total {total} must be <= budget");
    }

    #[test]
    fn zero_budget_returns_zeros() {
        let amounts = recommended_amounts(0, &[0.25, 0.1]);
        assert_eq!(amounts, vec![0, 0]);
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
}
