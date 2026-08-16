use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};

use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use monitor_loop::{
    RaceStatus, classify, count_started_before_post, has_result, warn_if_not_jst_now,
};
use paddock_domain::{
    BetCombination, HorseNum, HorseProbability, PairEvDiagnostic, PinnedSelection, Portfolio,
    PortfolioBet, PortfolioConfig, RECOMMENDED_MARKET_BLEND_ALPHA, Race, RaceId, TrackCondition,
    pair_ev_diagnostics,
};
use paddock_use_case::{PredictBetRecord, PredictSessionRecord, compose_portfolio};
use predict_format::{
    PortfolioFormat, format_explanations, format_portfolio, format_probs, format_probs_with_market,
    format_recent_runs_warning, surface_jp,
};

use crate::setup::App;

/// レース処理のモードフラグをまとめる（引数肥大の回避・#479）。
/// `explain` は予想根拠の表示（#274 由来）、`skip_all` は非対話一括スキップ（stdin を読まない・#479）。
#[derive(Debug, Clone, Copy)]
struct RaceRunOptions {
    explain: bool,
    skip_all: bool,
}

/// `run_session` が日単位で 1 度だけ引き当てる参照テーブル（レースごとの再クエリを避ける）。
/// `conditions` は記録済みの馬場入力（race_id 文字列 → 入力値・#80）、
/// `post_times` は race_cards 由来の発走時刻（#587）。どちらも欠落＝未記録/不明を意味する。
#[derive(Debug, Clone, Copy)]
struct DayLookups<'a> {
    conditions: &'a HashMap<String, Option<TrackCondition>>,
    post_times: &'a HashMap<RaceId, NaiveTime>,
}

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
    explain: bool,
    skip_all: bool,
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
    // 発走時刻は race_cards が正本（#391 と同じ一次ソース）。日単位で 1 度だけ引く（#587）。
    let post_times = app.interactor.post_times_by_date(date).await?;
    let lookups = DayLookups {
        conditions: &recorded,
        post_times: &post_times,
    };
    // 発走判定は post_time（JST 起算）と実行マシンの現在時刻の「時刻」を比べるだけなので、
    // TZ がずれると黙って狂う。当日か否かは見ない版で点検する（#587）。
    warn_if_not_jst_now("発走状態");
    print_session_header(&races, &post_times, date, Local::now().naive_local());
    let mut last_input: Option<TrackCondition> = None;
    let options = RaceRunOptions { explain, skip_all };

    for race in &races {
        if processed.contains(race.race_id.value()) {
            continue;
        }
        run_race(
            app,
            race,
            &mut session,
            race_budget,
            lookups,
            &mut last_input,
            options,
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
    lookups: DayLookups<'_>,
    last_input: &mut Option<TrackCondition>,
    options: RaceRunOptions,
) -> anyhow::Result<()> {
    let RaceRunOptions { explain, skip_all } = options;
    let recorded = lookups.conditions;
    // 上と同じくデバッグ時の pin（release では無効）。
    debug_assert_eq!(
        race.date, session.date,
        "races_by_date がセッション日以外の日付を返した"
    );
    println!();
    // 発走時刻と発走済み表示（#587）。対話・--skip-all は 1 日を跨いで動き続けるため、
    // 判定時刻はレースごとに取り直す（セッション開始時刻で固定しない）。
    println!(
        "{}",
        race_heading_for_day(race, lookups.post_times, Local::now().naive_local())
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
    // --skip-all（#479）は非対話。馬場入力プロンプトを出さずデフォルトを採用し、採用値を表示する
    // （対話時の read_track_condition と同じ default 決定・空入力採用の畳み方を stdin なしで再現）。
    // ここで決めた馬場条件は下の #80 ブロックで対話時と同様に保存されうる（表示のみ＝保存しない、ではない）。
    let track_condition = if skip_all {
        match default {
            Some(tc) => println!("馬場状態: {tc}（--skip-all: デフォルト採用）"),
            None => println!("馬場状態: 不明（--skip-all: デフォルト採用）"),
        }
        default
    } else {
        read_track_condition(&mut io::stdin().lock(), default)?
    };
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

    // 確率テーブル・市場比較・買い目推奨・EV 診断の表示は副作用のない `render_race_prediction` に
    // 委譲する（--overview の read-only 再表示と共有・#551）。返り値の portfolio を買い目入力に使う。
    let race_cap = race_budget.min(session.balance);
    let (portfolio, suggested) =
        match render_race_prediction(app, race, track_condition, race_cap, explain).await? {
            // 出馬表未登録（NotFound）はそのレースのみスキップ（Enter 待ちなし・現行挙動を踏襲）。
            RaceView::NoEntries => return Ok(()),
            // オッズ未取得はスキップのみ受付。--skip-all は Enter 待ちを省いて即次レースへ（#479）。
            RaceView::NoOdds => {
                if !skip_all {
                    let _ = read_line(&mut io::stdin().lock(), "Enter で次のレースへ > ")?;
                }
                return Ok(());
            }
            RaceView::Shown(portfolio) => {
                let suggested: Vec<u64> = portfolio.bets.iter().map(|b| b.stake).collect();
                (portfolio, suggested)
            }
        };

    println!();
    // --skip-all（#479）は購入方法プロンプトを読まず s（スキップ）相当で即次レースへ。
    // 買い目（bet_records）は記録しない（python ワンライナーの s 連打を置換）。馬場条件は上の
    // #80 ブロックで対話時同様に保存されうる点に注意（「一切記録しない」ではない）。
    if skip_all {
        println!("--skip-all: このレースはスキップします");
        return Ok(());
    }
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

    // 発走済みレースへの記録は確認を挟む（#623）。`s` と賭けなしを抜けた後＝実際に
    // `record_race_outcome` へ進む手前に置くので、スキップ運用に余計なプロンプトは出ず、
    // 長い払戻入力に入る前に止まれる。
    //
    // 取り直すのは **実行時刻だけ**（`post_times` は日単位・`race` はセッション開始時の
    // スナップショット）。見出し → オッズ再取得 → 馬場入力 → 金額編集の間に発走を跨ぐことがあり、
    // その分こそ「買えなかったのに記録される」からである。見出しに `[発走済]` が無いのに確認が
    // 出る場合があるが、プロンプトが発走時刻と判定時刻を併記するので理由は読める。判定そのものは
    // 見出しと同じ `started_state_for_day`（second source を作らない）。
    if !may_record_race(
        &mut io::stdin().lock(),
        race,
        lookups.post_times,
        Local::now().naive_local(),
    )? {
        println!("記録せず次のレースへ");
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
                bet_item.combination.label_ja(),
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

/// `render_race_prediction` の結果。表示の後に呼び出し側が取る分岐を伝える。
enum RaceView {
    /// 出馬表未登録（NotFound）。当該レースのみスキップ（Enter 待ちなし）。
    NoEntries,
    /// オッズ未取得。スキップのみ受付（対話時のみ Enter 待ち、--skip-all/--overview は即次へ）。
    NoOdds,
    /// 通常表示完了。表示に用いた買い目推奨（軸流しポートフォリオ）を返す。
    Shown(Portfolio),
}

/// 1 レースの予想ビュー（過去データ視点の確率テーブル・市場implied比較・買い目推奨・期待回収率・
/// 馬連vs馬単EV診断）を stdout に描画する。予想セッション状態（馬場保存・買い目記録・セッション更新）は
/// 書き込まない（ただしオッズは `app.odds.race_odds` の read-through 経由で、保存済みが不完全な
/// レースのみ再スクレイプして `race_odds` を更新しうる＝skip-all/対話と同じ副作用）。
/// run_race（対話/--skip-all）と run_overview（--overview 再表示・#551）で表示ロジックを共有し、
/// 重複と drift を防ぐ。`race_cap` は買い目推奨の予算上限、`track_condition` は予想に用いる馬場前提
/// （呼び出し側が解決済み）。
async fn render_race_prediction(
    app: &App,
    race: &Race,
    track_condition: Option<TrackCondition>,
    race_cap: u64,
    explain: bool,
) -> anyhow::Result<RaceView> {
    // 確率は 2 視点で取る（#272 確率分離）。順位付け（軸/相手）は blended（市場ブレンド・解像度が
    // 高い）、EV は pure（純モデル α=1.0・市場非依存）で計算し EV=P_blended×odds の循環を断つ。
    // --explain 時は根拠も返す（with_explanation）。
    let views = match app
        .interactor
        .predict_race_views(
            &race.race_id,
            RECOMMENDED_MARKET_BLEND_ALPHA,
            track_condition,
            explain,
        )
        .await
    {
        Ok(v) => v,
        // 出馬表未登録（NotFound）はそのレースのみスキップ。DB 障害等は継続不能なため伝播して中断する。
        Err(paddock_use_case::Error::NotFound(msg)) => {
            println!("出馬表が見つかりません（{msg}）。スキップします。");
            return Ok(RaceView::NoEntries);
        }
        Err(e) => return Err(e.into()),
    };

    // 近走データ皆無/過半欠損の警告（#552）。新馬戦・近走取得全滅は確率の信頼性が低いので、
    // 確率テーブルの前に注記して「回収率だけ見て候補入り」を防ぐ（表示自体は従来どおり続ける）。
    // render 共有により対話 predict・--skip-all・--overview のすべてで同じ警告が出る。
    if let Some(warn) = format_recent_runs_warning(
        views.recent_runs_coverage.field_size,
        views.recent_runs_coverage.horses_with_runs,
    ) {
        println!();
        println!("{warn}");
    }

    // 過去データ視点（#272 ④）: 純モデルの順位＋根拠。市場に依らない「公開データだけの読み」。
    println!();
    println!("【過去データ視点（純モデル）】");
    for line in format_probs(&views.pure) {
        println!("{line}");
    }
    if explain {
        for line in format_explanations(&views.pure, &views.explanations) {
            println!("{line}");
        }
    }

    // オッズ未取得（None）はスキップのみ受付。OddsInteractor が都度ライブスクレイプし、未公開は None に畳む。
    let Some(odds) = app.odds.race_odds(&race.race_id).await? else {
        println!();
        println!("オッズ未取得 — このレースはスキップします");
        return Ok(RaceView::NoOdds);
    };

    // 市場 implied との比較（過去データ視点に市場列を添える）。差＝純勝率−市場implied で割安/割高を読む。
    let market_win: HashMap<HorseNum, f64> =
        odds.win.iter().map(|(num, o)| (*num, o.value())).collect();
    // 条件依存枠バイアスの複勝 lift（#343・提示専用）。枠妙味フラグ（枠有利∧市場過小）の判定に使う。
    let gate_lift: HashMap<HorseNum, f64> = views
        .explanations
        .iter()
        .filter_map(|e| e.gate_bias_lift.map(|l| (e.horse_num, l)))
        .collect();
    println!();
    println!("【純モデル vs 市場implied】");
    for line in format_probs_with_market(&views.pure, &market_win, &gate_lift) {
        println!("{line}");
    }

    // 軸流しポートフォリオ（馬連＋ワイド＋三連複）を予算内・100 円単位で生成する。軸/相手は blended、
    // EV/的中は pure（循環断ち, #272）。上限は呼び出し側が決めた race_cap。配分・相手頭数は既定（#122）。
    let portfolio = compose_portfolio(&views, &odds, race_cap, &PinnedSelection::default());

    println!();
    println!("【市場EV視点：買い目推奨（軸流し, 予算¥{race_cap}/R・EV=純モデル×odds）】");
    // 軸/相手・混戦注記・各点の「そのまま買える形」整形は predict-watch と共有する
    // `predict_format::format_portfolio` に委譲する（#452）。predict は 2 スペースインデント・
    // 0 円脚も出す・未取得脚にも EV を付ける設定（現行出力をバイト単位で保つ）。軸なし・買い目なしの
    // 注記と期待回収率フッタは predict 固有なので前後に付す。
    if portfolio.axis.is_none() {
        println!("  確率推定が空のため買い目なし");
    }
    for line in format_portfolio(
        &portfolio,
        &PortfolioFormat {
            indent: "  ",
            skip_zero_stake: false,
            ev_on_unpriced: true,
        },
    ) {
        println!("{line}");
    }
    // `予算内で組める買い目なし` と format_portfolio の各点行は `bets.is_empty()` で排他
    // （空なら各点行ゼロ・非空なら本注記が fire しない）。よって注記を各点行の後に置いても順序は不変。
    if portfolio.bets.is_empty() {
        println!("  予算内で組める買い目なし");
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
            "  ポートフォリオ期待回収率 {:.1}% / 的中率 {:.1}% / 賭け計 ¥{}（モデル単独視点）{}",
            ev.roi * 100.0,
            ev.hit_prob * 100.0,
            portfolio.total_stake,
            note,
        );
    }

    // 馬連 vs 馬単(両方向) EV 診断（#246-C）。「穴は1着にならない」読みのとき本命→穴の馬単が
    // 同ペアの馬連より EV 優位になりうる。買い目選択の判断材料として並べて表示する。
    let diag = pair_ev_diagnostics(
        &views.blended,
        &views.pure,
        &odds,
        PortfolioConfig::default().partners,
    );
    print_pair_ev_diagnostics(diag.axis, &views.blended, &diag.rows);

    Ok(RaceView::Shown(portfolio))
}

/// EV 一覧を再表示する（--overview、#551）。予想セッション状態（セッション・買い目・馬場条件）は
/// 書き込まず、各レースの確率テーブル・買い目推奨・期待回収率を当日オッズで再計算して表示する。
/// --skip-all の一過性 stdout を `predict_sessions` の手動 DELETE なしで見返せるようにするのが狙い。
/// オッズは run_race と同じ read-through（`app.odds.race_odds`）で取得するため、保存済みが不完全な
/// レースは再スクレイプして `race_odds` を更新しうる（skip-all と同じ副作用・予想セッションには非干渉）。
///
/// 予算上限は各レース `race_budget`（残高で絞らない）。残高がレース予算以上のセッションでは
/// race_cap=race_budget が一致し朝の --skip-all 出力を再現するが、--budget を race_budget 未満で
/// 開始したセッションは skip-all 側が残高でクランプされるため買い目金額に差異が出うる。
/// 馬場前提は記録済み（--skip-all/対話が保存した値）→ races の確定値の順で解決するのみ（書かない）。
pub async fn run_overview(
    app: &App,
    date: NaiveDate,
    race_budget: u64,
    explain: bool,
) -> anyhow::Result<()> {
    let races = app.interactor.races_by_date(date).await?;
    let date_str = date.format("%Y-%m-%d").to_string();
    if races.is_empty() {
        println!("この日の開催はありません: {date_str}");
        return Ok(());
    }

    // 記録済みの馬場入力を読むだけ（書かない）。--skip-all/対話セッションが #80 で保存した値を
    // 引き当て、「どの馬場前提で予想したか」を再現する。未記録レースは races の確定値へフォールバック。
    let recorded: HashMap<String, Option<TrackCondition>> = app
        .interactor
        .find_predict_race_conditions(date)
        .await?
        .into_iter()
        .map(|r| (r.race_id.value().to_string(), r.track_condition))
        .collect();
    // 発走時刻は race_cards が正本（#391 と同じ一次ソース）。発走済みレースは除外せず区別する
    // ——除外すると #551 が意図した「完了済みセッションの見返し」が壊れるため（#587）。
    let post_times = app.interactor.post_times_by_date(date).await?;
    // 一覧の判定時刻は 1 回だけ取る（行ごとに時刻が動くと下の注記と食い違うため）。
    warn_if_not_jst_now("発走状態");
    let now = Local::now().naive_local();

    print_overview_header(&date_str, &races, &post_times, date, now);
    for race in &races {
        println!();
        // 発走判定は race.date、注記と不変条件チェックは --date が基準。races_by_date が
        // 日付で絞るので必ず一致する。debug_assert なので release（実運用バイナリ）では無効＝
        // デバッグ時の pin。本番の誤マーク検知は warn_if_result_before_post が担う。
        debug_assert_eq!(
            race.date, date,
            "races_by_date が --date 以外の日付を返した"
        );
        println!("{}", race_heading_for_day(race, &post_times, now));
        // 再表示は非対話。記録済み → 確定値 の順で馬場前提を解決する（対話の直前入力引き継ぎは
        // セッション内限定の概念のため使わない）。採用値を表示のみ（保存しない）。
        let track_condition = resolve_track_condition_default(
            recorded.get(race.race_id.value()).copied(),
            None,
            race.track_condition,
        );
        match track_condition {
            Some(tc) => println!("馬場状態: {tc}"),
            None => println!("馬場状態: 不明"),
        }
        // race_cap は残高で絞らない（セッション非依存の再表示のため）。表示結果は破棄する。
        let _ = render_race_prediction(app, race, track_condition, race_budget, explain).await?;
    }
    if let Some(footer) = overview_footer(date, now, Local::now().naive_local()) {
        println!();
        println!("{footer}");
    }
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
                &format!("  {} 推奨¥{} 入力額 > ", bet.combination.label_ja(), sug),
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

/// 軸-相手ペアの「馬連 vs 馬単(両方向)」EV 診断表（#246-C）。EV は的中確率 × オッズ、
/// オッズ未取得のセルは `—`。軸は `pair_ev_diagnostics` が決めた canonical な値を受け取り再計算しない。
fn print_pair_ev_diagnostics(
    axis: Option<HorseNum>,
    // `probs` は馬名の引き当てにのみ使う（勝率は表示しない）。EV は rows 側（純モデル由来）が持つため、
    // ここに blended/pure どちらを渡しても表示は変わらない（馬名は両系統で同一）。
    probs: &[HorseProbability],
    rows: &[PairEvDiagnostic],
) {
    if rows.is_empty() {
        return;
    }
    let name_of = |num| {
        probs
            .iter()
            .find(|p| p.horse_num == num)
            .map(|p| p.horse_name.value().to_string())
            .unwrap_or_default()
    };
    let fmt = |ev: f64, odds: Option<f64>| match odds {
        Some(o) => format!("{ev:.2}({o:.1})"),
        None => "—".to_string(),
    };
    println!();
    match axis {
        Some(a) => println!(
            "【馬連 vs 馬単 EV 診断（軸 {} {}）】",
            a.value(),
            name_of(a)
        ),
        None => return,
    }
    println!(
        "  {:<16} {:>14} {:>14} {:>14}",
        "相手", "馬連EV(オッズ)", "馬単 軸→相手", "馬単 相手→軸"
    );
    for r in rows {
        let label = format!("{} {}", r.partner.value(), name_of(r.partner));
        println!(
            "  {:<16} {:>14} {:>14} {:>14}",
            label,
            fmt(r.quinella_ev, r.quinella_odds),
            fmt(r.exacta_fwd_ev, r.exacta_fwd_odds),
            fmt(r.exacta_rev_ev, r.exacta_rev_odds),
        );
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

/// 発走済みなら確認に添える警告文を返す純関数（#623・テスト対象）。未発走なら `None`。
///
/// 表示を持たない——`result_before_post_warning` と同じ規律で、`println!` を抱えると文面を
/// assert できない。この文面は ADR 0087 決定 4 の拠り所（見出しと確認が食い違っても理由が
/// 読める）なので、テストで固定する価値がある。**発走判定と post_time の引き当てから文面までを
/// 1 本にする**のは、`started_state_for_day` の返り値をここで取りこぼしても bool しか見ない
/// テストでは気づけないため（発走時刻が常に不明と出る回帰が素通りする）。
///
/// **発走時刻と判定時刻を併記する**のは、この確認が見出しの `[発走済]` と一致しない場合があるため。
/// (1) 判定時刻は確認の直前に取り直すので見出しより後になる（その間に発走を跨いだ分を拾う）。
/// (2) `has_result` の不変条件が崩れたレースは**発走時刻が未来でも発走済みと判定される**
/// （`result_before_post_count` が別途 stderr で警告する既知の崩れ）。両方が見えれば
/// 「見出しでは未発走だったのになぜ聞かれたか」を人が判断できる。**両方に日付を付ける**のは、
/// 過去日の遡り入力では判定時刻（今日）と発走時刻（開催日）が別の日になるため。
/// 発走時刻不明は見出しと同じ `--:--` で表す。
fn started_race_record_notice(
    race: &Race,
    post_times: &HashMap<RaceId, NaiveTime>,
    now: NaiveDateTime,
) -> Option<String> {
    let (post_time, started) = started_state_for_day(race, post_times, now);
    started.then(|| {
        format!(
            "⚠ このレースは発走済みです（発走 {} {} / 判定時刻 {}）。",
            race.date.format("%m-%d"),
            format_post_time(post_time),
            now.format("%m-%d %H:%M")
        )
    })
}

/// 発走済みレースへ買い目を記録してよいかを尋ねる（#623）。`true` なら記録に進む。
///
/// 呼ぶのはゲート [`may_record_race`] だけ——判定を通さずここを直接呼ぶと全レースで確認が出る。
///
/// #587 の `[発走済]` は**見出しに出るだけ**で、購入方法プロンプトにも `record_race_outcome` にも
/// 効いていなかった。見落とすと「実際には買えなかったレースの買い目」が `predict_bets` に残り、
/// `--summary` や回収率の集計を汚す（`--resume` や夕方に前半レースを遡る運用で踏みやすい）。
///
/// **記録を禁止はしない**——発走後に「実際に買った分」を遡って入力する運用は正当なので、確認を
/// 経れば通す（ADR 0085 決定 2「除外ではなく区別」は維持し、記録の手前にゲートを 1 枚足すだけ）。
///
/// **既定は記録しない側**なので不正入力の再プロンプトは置かない（`y` 以外はすべて「記録しない」に
/// 畳む）。EOF も同じく `false`——`read_choice` の `s` / `read_u64` の 0 と同じ安全側への畳み方（#179）。
/// 出力先は stdout。診断ではなく対話の一部であり、この経路は対話セッション専用で
/// `scripts/predict-check` が読む `--skip-all` / `--overview` の stdout には現れない。
fn prompt_record_started_race<R: BufRead>(reader: &mut R, notice: &str) -> anyhow::Result<bool> {
    println!();
    println!("{notice}");
    let answer = read_line(
        reader,
        "買い目を記録しますか？ [y=記録する / それ以外=記録しない] > ",
    )?;
    Ok(matches!(answer.as_deref(), Some("y" | "Y")))
}

/// このレースの買い目を記録してよいか（#623）。`false` なら記録せずレースを抜ける。
/// 未発走なら確認せず `true`（stdin を 1 バイトも読まない）。
///
/// `run_race` から切り出したのは、**このゲートの配線こそ #623 の本体**だから。`run_race` 自体は
/// `App`（スクレイパが具象型）をモックできず単体テストが書けないので、判定 → 文面 → 確認 →
/// 記録可否の連結だけをここに閉じて `Cursor` で張る。
fn may_record_race<R: BufRead>(
    reader: &mut R,
    race: &Race,
    post_times: &HashMap<RaceId, NaiveTime>,
    now: NaiveDateTime,
) -> anyhow::Result<bool> {
    match started_race_record_notice(race, post_times, now) {
        None => Ok(true),
        Some(notice) => prompt_record_started_race(reader, &notice),
    }
}

/// 開催日と実行日の関係（#587）。日付軸の判定を 1 か所に集め、発走判定とヘッダ注記で共有する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeetingPhase {
    /// 開催日が過ぎている＝全レース発走済み（時刻を見るまでもない）。
    Over,
    /// 当日。発走済みかは時刻で決まる。
    Today,
    /// 開催日が未来＝全レース未発走（前日プリフェッチ運用で実際に起こる）。
    Ahead,
}

fn meeting_phase(date: NaiveDate, today: NaiveDate) -> MeetingPhase {
    match date.cmp(&today) {
        std::cmp::Ordering::Less => MeetingPhase::Over,
        std::cmp::Ordering::Equal => MeetingPhase::Today,
        std::cmp::Ordering::Greater => MeetingPhase::Ahead,
    }
}

/// `has_result` が依存する不変条件の崩れを検知して警告する（#587）。
///
/// `monitor_loop::has_result` は「発走前のレースは race_cards 由来で track_condition=NULL」という
/// `races_by_date` の不変条件に乗った早期シグナル。崩れると**発走前のレースに `[発走済]` が付く**
/// ——#587 が消そうとした誤読の逆向き（張れるレースを見送る）になる。監視側は同じ崩れを
/// `count_started_before_post` で警告しており（#459）、その防御を CLI にも置く。
///
/// 時刻比較は同日でしか意味を持たないので当日のみ点検する（過去日の見返しでは、結果取込済みかつ
/// `now.time() <= post_time` のレースが大量に該当してしまい、警告が総鳴りする）。
fn result_before_post_count(
    races: &[Race],
    post_times: &HashMap<RaceId, NaiveTime>,
    date: NaiveDate,
    now: NaiveDateTime,
) -> usize {
    if meeting_phase(date, now.date()) != MeetingPhase::Today {
        return 0;
    }
    let before_post = count_started_before_post(
        races,
        now.time(),
        |race: &Race| post_times.get(&race.race_id).copied(),
        has_result,
    );
    // monitor-loop の防御は post_time があるレースしか数えない（classify は post_time 不明を
    // Unknown として扱い、監視の対象外にするため）。だが CLI は post_time 不明でも
    // `result_present` だけで [発走済] を出す（is_started_at）。その **CLI 固有の経路** にも
    // 同じ防御を効かせないと、一番見えにくい組み合わせだけ警告なしで誤マークが付く。
    let missing_post = races
        .iter()
        .filter(|race| has_result(race) && !post_times.contains_key(&race.race_id))
        .count();
    before_post + missing_post
}

/// [`result_before_post_count`] が 1 件以上なら警告文を返す純関数（#587・テスト対象）。
/// 表示は呼び出し側（`println!` を持つと「警告を出す条件」を assert できない）。
fn result_before_post_warning(
    races: &[Race],
    post_times: &HashMap<RaceId, NaiveTime>,
    date: NaiveDate,
    now: NaiveDateTime,
) -> Option<String> {
    let broken = result_before_post_count(races, post_times, date, now);
    (broken > 0).then(|| {
        format!(
            "⚠ 発走前なのに結果が取り込まれているレースが {broken} 件あります。\
             これらは実際には未発走でも [発走済] と表示されます。"
        )
    })
}

/// 上の警告を出す。
///
/// 出力先は **stderr**。stdout は `scripts/predict-check` が機械パースするデータチャネルなので、
/// 診断メッセージを混ぜない（現行の見出し regex では素通りするが、混ぜない方が安全）。
fn warn_if_result_before_post(
    races: &[Race],
    post_times: &HashMap<RaceId, NaiveTime>,
    date: NaiveDate,
    now: NaiveDateTime,
) {
    if let Some(msg) = result_before_post_warning(races, post_times, date, now) {
        eprintln!("{msg}");
    }
}

/// 実行時刻 `now` の時点でそのレースが発走済みかを判定する純関数（#587）。
///
/// 時刻軸の判定は監視側と同じ [`classify`]（`monitor-loop`）に委譲し、ここは classify が持たない
/// 日付軸だけを畳む（classify は `NaiveTime` のみで日付を持たないため、過去日を `--overview` すると
/// 発走前に見えてしまう）。`window: None` は windowless 判定＝「発走済みか否か」に落ちる。
///
/// - 開催日が過ぎている → 発走時刻が不明でも発走済み（日付が過ぎた事実で言い切れる）
/// - 開催日が未来 → 未発走
/// - 当日 → 結果取込済みなら発走済み、でなければ `classify` に委譲
///   （`now == post_time` はまだ未発走）
///
/// 当日の post_time 不明は `Unknown` となり発走済みと断定しない（`web-spa.md` の方針と同じ）。
/// ただし結果が入っていれば発走済みと言い切れるので、`result_present` は `classify` より前に見る
/// ——`classify` は post_time が `None` の時点で `has_result` を見ずに `Unknown` を返す
/// （監視側は「発走時刻不明＝収集対象外」で足りるため）。時刻軸の判定自体は `classify` のまま。
///
/// `result_present` は `monitor_loop::has_result`（`track_condition` か着順の有無）を想定する。
/// これは「発走前のレースは `race_cards` 由来で track_condition=NULL」という `races_by_date` の
/// 不変条件に乗った早期シグナルで、崩れると発走前に `[発走済]` が付きうる。
fn is_started_at(
    target: NaiveDate,
    now: NaiveDateTime,
    post_time: Option<NaiveTime>,
    result_present: bool,
) -> bool {
    match meeting_phase(target, now.date()) {
        MeetingPhase::Over => true,
        MeetingPhase::Ahead => false,
        // 結果はここで見終わっているので、classify には時刻軸だけを判定させる（第 3 引数は false）。
        MeetingPhase::Today => {
            result_present || classify(now.time(), post_time, false, None) == RaceStatus::Started
        }
    }
}

/// 日単位の発走時刻マップから、そのレースの `(発走時刻, 発走済みか)` を 1 回で引く（#587 / #623）。
///
/// post_time の引き当てと [`is_started_at`] の呼び出しをここ 1 箇所に閉じ、見出しの `[発走済]`
/// （`race_heading_for_day`）と記録確認の文面（`started_race_record_notice`。ゲートは
/// `may_record_race`）が**同じ判定**を通るようにする。#623 の要件「判定の second source を
/// 作らない」はこの共有点で担保する。
/// **引き当ての結果も返す**のは、呼び出し側が表示用に `post_times.get` をもう一度書けば
/// 発走時刻の持ち方を変えたとき片方だけ直る形が残るため。返り値の順は
/// `(引き当てた発走時刻, 発走済みか)` で固定（位置分解で受ける契約なので、要素を足すときは
/// 呼び出し 2 箇所——`race_heading_for_day` と `started_race_record_notice`——を必ず見直す）。
fn started_state_for_day(
    race: &Race,
    post_times: &HashMap<RaceId, NaiveTime>,
    now: NaiveDateTime,
) -> (Option<NaiveTime>, bool) {
    let post_time = post_times.get(&race.race_id).copied();
    (
        post_time,
        is_started_at(race.date, now, post_time, has_result(race)),
    )
}

/// 日単位の発走時刻マップから、そのレースの見出し 1 行を組み立てる（#587）。
/// post_time の引き当て → 発走判定 → 見出し文字列を 1 本にまとめ、`run_race` と `run_overview`
/// の両経路で共有する（引き当てを取り違えても片方だけ壊れる、という形にしないため）。
fn race_heading_for_day(
    race: &Race,
    post_times: &HashMap<RaceId, NaiveTime>,
    now: NaiveDateTime,
) -> String {
    let (post_time, started) = started_state_for_day(race, post_times, now);
    race_heading(race, post_time, started)
}

/// EV 一覧のヘッダに出す注記を組み立てる純関数（#587）。
///
/// 当日以外は `[発走済]` が日付だけで決まるので、時刻を書くと誤読になる。過去日に
/// 「HH:MM 時点の判定」とだけ書くと、実行時刻より後の発走時刻にも `[発走済]` が付く理由が
/// 読めない（例: 8/15 10:02 に 8/9 を見返すと 12:25 発走にも `[発走済]` が付く）。未来日は
/// 逆に 1 件もマークが付かないので、判定時刻の説明そのものが空振りする。
///
/// 当日だけ判定時刻を**日付込み**で出す。これは一覧全体を貫く基準時刻＝作成開始時刻であって、
/// 実行が終わった時刻ではない（オッズ再取得を伴うと数分かかり、その間に発走した分は反映されない）。
fn overview_note(date: NaiveDate, now: NaiveDateTime) -> String {
    phase_note(meeting_phase(date, now.date()), Some(now))
}

/// EV 一覧のヘッダ（見出し行＋注記）の行並び（#587・テスト対象）。
fn overview_header_lines(
    date_str: &str,
    race_count: usize,
    date: NaiveDate,
    now: NaiveDateTime,
) -> Vec<String> {
    vec![
        format!("=== {date_str} EV 一覧（再表示・読み取り専用） — {race_count} レース ==="),
        overview_note(date, now),
    ]
}

/// EV 一覧のヘッダを出し、続けて不変条件の警告を出す（#587）。
///
/// **警告の呼び出しをここ 1 か所に閉じる**のが要点。出力順を直すときに元の呼び出しを消し忘れて
/// 同じ警告が 2 度出る事故を実際に踏んだので、呼び出し箇所が 1 つしかない形にした。
fn print_overview_header(
    date_str: &str,
    races: &[Race],
    post_times: &HashMap<RaceId, NaiveTime>,
    date: NaiveDate,
    now: NaiveDateTime,
) {
    for line in overview_header_lines(date_str, races.len(), date, now) {
        println!("{line}");
    }
    warn_if_result_before_post(races, post_times, date, now);
}

/// EV 一覧の末尾に出す完了注記（#587）。当日以外は `None`（時刻の説明が意味を持たない）。
///
/// 当日はオッズ read-through で一覧の作成に数分かかることがあり、その間に発走したレースは
/// 未発走のまま出ている。開始と完了の差を見せて「いつ時点の判定か」を読み手に渡す。
/// phase の判定は**完了時刻**で行う——日跨ぎの一覧で「完了 00:12」と出しながら当日扱いを
/// 続ける、といった自己矛盾を避けるため。
fn overview_footer(
    date: NaiveDate,
    started_at: NaiveDateTime,
    finished_at: NaiveDateTime,
) -> Option<String> {
    if meeting_phase(date, started_at.date()) != MeetingPhase::Today {
        return None;
    }
    // 日を跨いだ実行こそ「判定基準は開始時刻のまま」が最も効く場面なので、消さずに日付込みで出す。
    let fmt = if started_at.date() == finished_at.date() {
        "%H:%M"
    } else {
        "%Y-%m-%d %H:%M"
    };
    Some(format!(
        "※ 一覧作成完了 {}（判定基準は開始時刻 {} のまま。この間に発走した分は未発走表示）",
        finished_at.format(fmt),
        started_at.format(fmt)
    ))
}

/// 注記の文言（#587）。当日以外の 2 文は `--overview` と対話で共通なので、ここに 1 本化する
/// （見出しで潰したのと同じ drift をここで作らない）。`at` は当日の基準時刻——`Some` なら
/// その時刻で一覧全体を判定したこと、`None` ならレースごとに判定し直すことを意味する。
fn phase_note(phase: MeetingPhase, at: Option<NaiveDateTime>) -> String {
    match (phase, at) {
        (MeetingPhase::Over, _) => "※ この開催は終了しています（全レース発走済）".to_string(),
        (MeetingPhase::Ahead, _) => {
            "※ この開催はまだ実施されていません（全レース未発走）".to_string()
        }
        (MeetingPhase::Today, Some(at)) => format!(
            "※ 一覧作成開始 {} 時点の判定。[発走済] はその時刻に発走済み（結果確定の有無とは別）",
            at.format("%Y-%m-%d %H:%M")
        ),
        (MeetingPhase::Today, None) => {
            "※ [発走済] は表示時点で発走済み（結果確定の有無とは別）".to_string()
        }
    }
}

/// 対話セッション（対話 / `--skip-all`）のヘッダに出す注記（#587）。
///
/// `[発走済]` の基準を `--overview` だけでなくこちらでも明示する（マークだけ配って但し書きを
/// 配らない非対称にしない・ADR 0085 決定 5）。判定時刻はレースごとに取り直すので、
/// 当日は一覧のような基準時刻を書かない。
///
/// この注記はセッション開始時に 1 度だけ出す。対話が日を跨ぐと（前夜起動など）注記は開始時点の
/// ままになるが、各レースのマークはレースごとに取り直した現在時刻で判定される。当日の文言が
/// 「表示時点で」と時刻を名指ししないのはこのため。**前夜起動（`Ahead`）で 0 時を回った場合は
/// 「全レース未発走」の注記のまま各行に `[発走済]` が付きうる**——行の判定が正しく、注記だけが
/// 開始時点の事実であることに注意（レース毎に注記を出し直すほどの実害はないと判断した）。
fn session_note(date: NaiveDate, now: NaiveDateTime) -> String {
    phase_note(meeting_phase(date, now.date()), None)
}

/// 対話セッションのヘッダ注記と不変条件の警告を出す（#587）。
/// `--overview` 側と同じく、警告の呼び出しをこの 1 か所に閉じる。
fn print_session_header(
    races: &[Race],
    post_times: &HashMap<RaceId, NaiveTime>,
    date: NaiveDate,
    now: NaiveDateTime,
) {
    println!("{}", session_note(date, now));
    warn_if_result_before_post(races, post_times, date, now);
}

/// 発走時刻の表示整形（#587 / #623）。不明は見出しと同じ `--:--` に落とす。
/// 見出し（[`race_heading`]）と発走済み確認の文面（[`started_race_record_notice`]）が
/// **同じ表記**を使うための共有点——プレースホルダを変えたときに片方だけ直る形にしない。
fn format_post_time(post_time: Option<NaiveTime>) -> String {
    post_time.map_or_else(|| "--:--".to_string(), |t| t.format("%H:%M").to_string())
}

/// レース見出しの 1 行を組み立てる純関数（#587）。`run_race`（対話 / `--skip-all`）と
/// `run_overview` で共有し、同一フォーマットの重複による drift を防ぐ。
///
/// 発走時刻は常に出し（不明は `--:--`・整形は [`format_post_time`]）、発走済みのときだけ
/// `[発走済]` を付ける。
fn race_heading(race: &Race, post_time: Option<NaiveTime>, started: bool) -> String {
    let post = format_post_time(post_time);
    let started_mark = if started { "[発走済] " } else { "" };
    format!(
        "--- レース {}: {} {} {}m（発走 {post}）{started_mark}---",
        race.race_num,
        race.venue.as_jp(),
        surface_jp(race.surface),
        race.distance
    )
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
        is_started_at, make_bet_record, may_record_race, overview_footer, overview_header_lines,
        overview_note, prompt_record_started_race, race_heading, race_heading_for_day, read_choice,
        read_edited_amounts, read_track_condition, read_u64, resolve_track_condition_default,
        result_before_post_count, result_before_post_warning, session_note,
        started_race_record_notice, started_state_for_day,
    };
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    use paddock_domain::horse_result::HorseNum;
    use paddock_domain::{
        BetCombination, BetMethod, PortfolioBet, Race, RaceId, Surface, TrackCondition, Venue,
    };
    use std::collections::HashMap;
    use std::io::Cursor;

    fn horse(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    /// 開催日 2026-08-09・新潟 芝 2000m のレース（発走時刻は Race に持たないので引数で渡す）。
    /// race_id は race_num と揃える——`race_heading_for_day` は race_id をキーに発走時刻を引くため、
    /// ここが固定だと「別レースの発走時刻を引いても気づけない」テストになる。
    fn race(race_num: u32) -> Race {
        Race {
            race_id: RaceId::try_from(format!("2026-2-niigata-4-{race_num}R")).unwrap(),
            date: race_date(),
            venue: Venue::Niigata,
            round: 2,
            day: 4,
            race_num,
            surface: Surface::Turf,
            distance: 2000,
            track_condition: None,
            weather: None,
            results: vec![],
        }
    }

    fn race_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 9).unwrap()
    }

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    /// 開催日当日の実行時刻。
    fn today_at(h: u32, m: u32) -> NaiveDateTime {
        race_date().and_time(t(h, m))
    }

    /// 開催月（2026-08）の任意日の実行時刻。開催日 (9 日) との前後で日付軸の枝を選ぶ。
    fn day_at(day: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 8, day)
            .unwrap()
            .and_time(t(h, m))
    }

    #[test]
    fn heading_marks_started_race() {
        assert_eq!(
            race_heading(&race(1), Some(t(9, 40)), true),
            "--- レース 1: 新潟 芝 2000m（発走 09:40）[発走済] ---"
        );
    }

    #[test]
    fn heading_omits_mark_for_upcoming_race() {
        // 未発走はマークを付けない（[未発走] は出さない）。
        assert_eq!(
            race_heading(&race(5), Some(t(12, 25)), false),
            "--- レース 5: 新潟 芝 2000m（発走 12:25）---"
        );
    }

    #[test]
    fn heading_shows_dashes_when_post_time_unknown() {
        // post_time 不明は --:-- とし、発走済みとは断定しない（web-spa と同方針）。
        assert_eq!(
            race_heading(&race(8), None, false),
            "--- レース 8: 新潟 芝 2000m（発走 --:--）---"
        );
    }

    #[test]
    fn heading_marks_started_even_when_post_time_unknown() {
        // 過去日は post_time 不明でも発走済み（is_started_at の Less 分岐）。その組み合わせも
        // 見出しとして成立することを固定する。
        assert_eq!(
            race_heading(&race(8), None, true),
            "--- レース 8: 新潟 芝 2000m（発走 --:--）[発走済] ---"
        );
    }

    /// 生成側（Rust）と解析側（Python）が同じ見出しを見ていることを固定する golden（#587）。
    /// `include_str!` なのでファイルが消えればコンパイルが通らない。詳細は
    /// `src/apps/predict/testdata/README.md`。
    const HEADER_GOLDEN: &str = include_str!("../testdata/pred_header_samples.txt");

    #[test]
    fn heading_samples_match_the_shared_golden() {
        // 解析側（scripts/predict-check）はこのファイルをパースできることを張っている。
        // 見出しを変えたのに golden を直さなければここで落ち、直せば Python 側が落ちる
        // ——言語をまたいだ契約のズレを、どちらかのテストで必ず捕まえるための結び目。
        let lines: Vec<&str> = HEADER_GOLDEN.lines().collect();
        // 長さを先に見る（行が減ったとき index out of bounds ではなく件数のズレとして落とす）。
        assert_eq!(lines.len(), 5, "golden の行数が変わった: {lines:?}");
        assert_eq!(lines[0], race_heading(&race(1), Some(t(9, 40)), true));
        assert_eq!(lines[1], race_heading(&race(5), Some(t(12, 25)), false));
        assert_eq!(lines[2], race_heading(&race(8), None, false));
        // 発走時刻不明 × 発走済（過去日の見返しで card に post_time が無いときの通常形）。
        // `--:--` はハイフンを含むうえマークも付くので、解析側が最も落としやすい組み合わせ。
        assert_eq!(lines[3], race_heading(&race(9), None, true));
        // 5 行目は #587 以前の旧形式。Rust はもう生成しないので、ここでは生成物と比較しない
        // （解析側だけが後方互換のために使う）。
        assert_eq!(lines[4], "--- レース 1: 東京 芝 1600m ---");
    }

    #[test]
    fn heading_for_day_looks_up_post_time_by_race_id() {
        // 引き当て（race_id → post_time）から見出しまでの配線を張る。別レースの発走時刻を
        // 引いていないこと・マップに無いレースが --:-- になることを同時に見る。
        let post_times: HashMap<RaceId, NaiveTime> =
            [(race(1).race_id, t(9, 40)), (race(5).race_id, t(12, 25))]
                .into_iter()
                .collect();
        let now = today_at(10, 0);
        assert_eq!(
            race_heading_for_day(&race(1), &post_times, now),
            "--- レース 1: 新潟 芝 2000m（発走 09:40）[発走済] ---"
        );
        assert_eq!(
            race_heading_for_day(&race(5), &post_times, now),
            "--- レース 5: 新潟 芝 2000m（発走 12:25）---"
        );
        assert_eq!(
            race_heading_for_day(&race(8), &post_times, now),
            "--- レース 8: 新潟 芝 2000m（発走 --:--）---"
        );
    }

    #[test]
    fn overview_note_states_the_meeting_is_over_for_past_dates() {
        // 過去日の見返しでは [発走済] は日付で決まる。実行時刻を書くと「10:02 なのに 12:25 発走が
        // 発走済」と読めてしまうため、時刻ではなく開催が終わっている旨を出す。
        let now = race_date().succ_opt().unwrap().and_time(t(10, 2));
        assert_eq!(
            overview_note(race_date(), now),
            "※ この開催は終了しています（全レース発走済）"
        );
    }

    #[test]
    fn overview_note_shows_full_timestamp_only_for_today() {
        // 当日は判定時刻を日付込みで出す（開催日との一致が読めるように）。
        assert_eq!(
            overview_note(race_date(), today_at(10, 5)),
            "※ 一覧作成開始 2026-08-09 10:05 時点の判定。[発走済] はその時刻に発走済み（結果確定の有無とは別）"
        );
    }

    #[test]
    fn overview_note_states_the_meeting_is_ahead_for_future_dates() {
        // 未来日（前日プリフェッチ）は 1 件もマークが付かないので、判定時刻の説明は空振りする。
        let day_before = race_date().pred_opt().unwrap().and_time(t(22, 0));
        assert_eq!(
            overview_note(race_date(), day_before),
            "※ この開催はまだ実施されていません（全レース未発走）"
        );
    }

    #[test]
    fn result_before_post_is_counted_only_for_today() {
        // 発走前（now <= post）なのに結果取込済み＝ races_by_date の不変条件が崩れた兆候。
        let post_times: HashMap<RaceId, NaiveTime> =
            [(race(1).race_id, t(15, 0))].into_iter().collect();
        let mut broken = race(1);
        broken.track_condition = Some(TrackCondition::Good); // has_result = true
        let races = vec![broken];

        // 当日・発走前 → 検知する。
        assert_eq!(
            result_before_post_count(&races, &post_times, race_date(), today_at(10, 0)),
            1
        );
        // 当日・発走後 → 正常な遷移なので 0。
        assert_eq!(
            result_before_post_count(&races, &post_times, race_date(), today_at(16, 0)),
            0
        );
        // 過去日の見返しは時刻比較が無意味。ここが 0 にならないと、見返しのたびに警告が総鳴りする。
        let next_day = race_date().succ_opt().unwrap().and_time(t(10, 0));
        assert_eq!(
            result_before_post_count(&races, &post_times, race_date(), next_day),
            0
        );
    }

    #[test]
    fn result_without_post_time_is_counted_too() {
        // monitor-loop の防御は post_time があるレースしか数えないが、CLI は post_time 不明でも
        // 結果があれば [発走済] を出す。その CLI 固有の経路も検知できないと、一番見えにくい
        // 組み合わせだけ警告なしで誤マークが付く。
        let mut broken = race(1);
        broken.track_condition = Some(TrackCondition::Good);
        let races = vec![broken];
        assert_eq!(
            result_before_post_count(&races, &HashMap::new(), race_date(), today_at(10, 0)),
            1
        );
        // 結果が無ければ（＝ post_time 不明なだけ）異常ではない。
        assert_eq!(
            result_before_post_count(&[race(1)], &HashMap::new(), race_date(), today_at(10, 0)),
            0
        );
    }

    #[test]
    fn result_before_post_warning_mentions_the_count() {
        let mut broken = race(1);
        broken.track_condition = Some(TrackCondition::Good);
        let races = vec![broken];
        let msg = result_before_post_warning(&races, &HashMap::new(), race_date(), today_at(10, 0))
            .expect("崩れていれば警告する");
        assert!(msg.contains("1 件"), "{msg}");
        assert!(msg.contains("[発走済]"), "{msg}");
        // 健全なら警告しない（＝毎回鳴るノイズにならない）。
        assert_eq!(
            result_before_post_warning(&[race(1)], &HashMap::new(), race_date(), today_at(10, 0)),
            None
        );
    }

    #[test]
    fn session_note_switches_by_meeting_phase() {
        // 対話 / --skip-all 側。当日は基準時刻を書かない（判定はレースごとに取り直すため）。
        assert_eq!(
            session_note(race_date(), today_at(10, 0)),
            "※ [発走済] は表示時点で発走済み（結果確定の有無とは別）"
        );
        assert_eq!(
            session_note(
                race_date(),
                race_date().succ_opt().unwrap().and_time(t(10, 0))
            ),
            "※ この開催は終了しています（全レース発走済）"
        );
        assert_eq!(
            session_note(
                race_date(),
                race_date().pred_opt().unwrap().and_time(t(22, 0))
            ),
            "※ この開催はまだ実施されていません（全レース未発走）"
        );
    }

    #[test]
    fn overview_header_lines_are_title_then_note() {
        // ヘッダの並び（見出し行 → 注記）を固定する。警告は stderr なのでここには入らない。
        let lines = overview_header_lines("2026-08-09", 35, race_date(), today_at(10, 6));
        assert_eq!(
            lines,
            vec![
                "=== 2026-08-09 EV 一覧（再表示・読み取り専用） — 35 レース ===".to_string(),
                overview_note(race_date(), today_at(10, 6)),
            ]
        );
    }

    #[test]
    fn overview_footer_reports_elapsed_only_for_today() {
        // 当日は開始と完了の差を出す（実行中に発走した分は未発走のまま出ているため）。
        assert_eq!(
            overview_footer(race_date(), today_at(10, 6), today_at(10, 11)).as_deref(),
            Some(
                "※ 一覧作成完了 10:11（判定基準は開始時刻 10:06 のまま。この間に発走した分は未発走表示）"
            )
        );
        // 過去日の見返しでは時刻の説明が意味を持たない。
        let next_day = race_date().succ_opt().unwrap().and_time(t(10, 0));
        assert_eq!(overview_footer(race_date(), next_day, next_day), None);
        // 日跨ぎ（開始 23:50 / 完了 00:12）こそ「判定基準は開始時刻のまま」が最も効く場面なので、
        // 消さずに日付込みで出す。
        let started = race_date().and_time(t(23, 50));
        let finished = race_date().succ_opt().unwrap().and_time(t(0, 12));
        assert_eq!(
            overview_footer(race_date(), started, finished).as_deref(),
            Some(
                "※ 一覧作成完了 2026-08-10 00:12（判定基準は開始時刻 2026-08-09 23:50 のまま。この間に発走した分は未発走表示）"
            )
        );
    }

    #[test]
    fn started_when_race_date_has_passed() {
        // 開催日が過ぎていれば時刻を見るまでもなく発走済み（post_time 不明でも同じ）。
        let now = race_date().succ_opt().unwrap().and_time(t(9, 0));
        assert!(is_started_at(race_date(), now, Some(t(15, 45)), false));
        assert!(is_started_at(race_date(), now, None, false));
    }

    #[test]
    fn not_started_when_race_date_is_in_the_future() {
        // 翌日開催を前日に見た場合、時刻の大小に関わらず未発走。
        let now = race_date().pred_opt().unwrap().and_time(t(23, 0));
        assert!(!is_started_at(race_date(), now, Some(t(9, 40)), false));
        assert!(!is_started_at(race_date(), now, None, false));
    }

    #[test]
    fn today_started_only_after_post_time() {
        // 当日は classify に委譲する。発走時刻ちょうどはまだ未発走（classify の境界と同じ）。
        assert!(!is_started_at(
            race_date(),
            today_at(9, 39),
            Some(t(9, 40)),
            false
        ));
        assert!(!is_started_at(
            race_date(),
            today_at(9, 40),
            Some(t(9, 40)),
            false
        ));
        assert!(is_started_at(
            race_date(),
            today_at(9, 41),
            Some(t(9, 40)),
            false
        ));
    }

    #[test]
    fn today_started_when_result_present() {
        // 結果取込済みは発走時刻前でも発走済み（classify の早期シグナル）。
        assert!(is_started_at(
            race_date(),
            today_at(9, 0),
            Some(t(9, 40)),
            true
        ));
    }

    #[test]
    fn today_unknown_post_time_is_not_marked_started() {
        // post_time 不明（当日）は判定不能。発走済みと断定しない。
        assert!(!is_started_at(race_date(), today_at(23, 0), None, false));
    }

    #[test]
    fn today_result_present_wins_over_unknown_post_time() {
        // post_time 不明でも結果が入っていれば発走済み。classify は post_time が None の時点で
        // has_result を見ずに Unknown を返すので、その手前で拾えていることを固定する。
        assert!(is_started_at(race_date(), today_at(9, 0), None, true));
    }

    #[test]
    fn heading_for_day_uses_result_when_post_time_is_missing() {
        // 上を見出し側からも張る（引き当てに無い＝時刻不明でも [発走済] が付く）。
        let mut race = race(3);
        race.track_condition = Some(TrackCondition::Good);
        assert_eq!(
            race_heading_for_day(&race, &HashMap::new(), today_at(9, 0)),
            "--- レース 3: 新潟 芝 2000m（発走 --:--）[発走済] ---"
        );
    }

    #[test]
    fn started_state_for_day_agrees_with_the_heading_marker() {
        // #623 の記録確認は見出しの [発走済] と同じ判定を通らねばならない（second source 禁止）。
        // 判定が分岐すると「見出しは未発走なのに毎レース確認が出る」等の齟齬が静かに生まれる。
        // **期待値そのものも各ケースに書く**——一致だけを見ると `started_state_for_day` を
        // 「常に false」に変異させても見出し側が道連れで false になり、テストが素通りするため。
        let mut with_result = race(2);
        with_result.track_condition = Some(TrackCondition::Good);
        let post_times = HashMap::from([(race(1).race_id, t(9, 40))]);
        let cases = [
            (race(1), today_at(9, 0), false),    // 当日・発走前
            (race(1), today_at(9, 41), true),    // 当日・発走後
            (race(2), today_at(23, 0), false),   // 当日・post_time 不明
            (with_result, today_at(9, 0), true), // 当日・post_time 不明だが結果あり
            (race(1), day_at(10, 9, 0), true),   // 過去開催（時刻を見ずに発走済み）
            (race(1), day_at(8, 23, 0), false),  // 未来開催（時刻を見ずに未発走）
        ];
        for (target, now, expected) in cases {
            let (_, started) = started_state_for_day(&target, &post_times, now);
            assert_eq!(
                started,
                expected,
                "発走判定が期待と違う: {} / {now}",
                target.race_id.value()
            );
            assert_eq!(
                race_heading_for_day(&target, &post_times, now).contains("[発走済]"),
                expected,
                "見出しの [発走済] と記録確認の判定がずれた: {} / {now}",
                target.race_id.value()
            );
        }
    }

    #[test]
    fn started_race_record_notice_shows_both_post_time_and_decision_time() {
        // 見出しの [発走済] と確認が食い違いうる（判定時刻を取り直す・has_result の不変条件崩れ）
        // ことの唯一の手掛かりが文面なので、両方の時刻が日付付きで出ることを固定する（ADR 0087
        // 決定 4）。発走時刻の引き当てが文面まで届いていること（--:-- に落ちないこと）も兼ねる。
        let post_times = HashMap::from([(race(1).race_id, t(9, 40))]);
        assert_eq!(
            started_race_record_notice(&race(1), &post_times, day_at(16, 15, 1)).as_deref(),
            Some("⚠ このレースは発走済みです（発走 08-09 09:40 / 判定時刻 08-16 15:01）。")
        );
    }

    #[test]
    fn started_race_record_notice_marks_unknown_post_time() {
        // post_time 不明でも結果取込済みなら発走済みと判定される。時刻は見出しと同じ --:-- で表す。
        let mut with_result = race(2);
        with_result.track_condition = Some(TrackCondition::Good);
        assert_eq!(
            started_race_record_notice(&with_result, &HashMap::new(), today_at(9, 0)).as_deref(),
            Some("⚠ このレースは発走済みです（発走 08-09 --:-- / 判定時刻 08-09 09:00）。")
        );
    }

    #[test]
    fn started_race_record_notice_is_absent_for_upcoming_races() {
        // 未発走に文面は出ない（#623 は記録の手前の 1 枚であって全レースの確認ではない）。
        let post_times = HashMap::from([(race(1).race_id, t(9, 40))]);
        assert!(started_race_record_notice(&race(1), &post_times, today_at(9, 0)).is_none());
    }

    #[test]
    fn may_record_race_passes_upcoming_races_without_reading_stdin() {
        // 未発走は確認せず通す。入力を与えたうえで position() が 0 のままであることを見て、
        // 「読まなかった」を「読んで EOF を得た」と取り違えないようにする。
        let post_times = HashMap::from([(race(1).race_id, t(9, 40))]);
        let mut input = Cursor::new(b"n\n".to_vec());
        assert!(may_record_race(&mut input, &race(1), &post_times, today_at(9, 0)).unwrap());
        assert_eq!(input.position(), 0, "未発走レースで stdin を消費した");
    }

    #[test]
    fn may_record_race_blocks_started_races_by_default() {
        // 発走済みなら確認に入り、既定（EOF）は記録しない。ゲートの配線そのものを張る。
        let post_times = HashMap::from([(race(1).race_id, t(9, 40))]);
        let mut eof = Cursor::new(Vec::new());
        assert!(!may_record_race(&mut eof, &race(1), &post_times, today_at(9, 41)).unwrap());
        let mut yes = Cursor::new(b"y\n".to_vec());
        assert!(may_record_race(&mut yes, &race(1), &post_times, today_at(9, 41)).unwrap());
    }

    #[test]
    fn prompt_record_started_race_defaults_to_not_recording_on_eof() {
        // EOF は「記録しない」へ畳む（#179 の安全側規律。read_choice の s / read_u64 の 0 と同じ）。
        let mut input = Cursor::new(Vec::new());
        assert!(!prompt_record_started_race(&mut input, "⚠").unwrap());
    }

    #[test]
    fn prompt_record_started_race_treats_blank_as_not_recording() {
        // 既定は記録しない側。Enter 空打ちで記録に進んでしまうと確認の意味が無い。
        let mut input = Cursor::new(b"\n".to_vec());
        assert!(!prompt_record_started_race(&mut input, "⚠").unwrap());
    }

    #[test]
    fn prompt_record_started_race_accepts_y() {
        // 記録自体は禁止しない（発走後に実際に買った分を遡って入力する運用は正当・#623）。
        let mut input = Cursor::new(b"y\n".to_vec());
        assert!(prompt_record_started_race(&mut input, "⚠").unwrap());
        let mut upper = Cursor::new(b"Y\n".to_vec());
        assert!(prompt_record_started_race(&mut upper, "⚠").unwrap());
    }

    #[test]
    fn prompt_record_started_race_rejects_other_input_without_reprompting() {
        // y 以外はすべて 1 回で「記録しない」に落とす（既定が安全側なので再プロンプトを置かない）。
        // 2 行目を消費していないことまで見て、再プロンプトのループが無いことを固定する。
        let mut input = Cursor::new(b"yes\ny\n".to_vec());
        assert!(!prompt_record_started_race(&mut input, "⚠").unwrap());
        assert!(prompt_record_started_race(&mut input, "⚠").unwrap());
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
                method: BetMethod::Nagashi,
                stake: 500,
                odds: None,
                ev: 0.0,
                hit_prob: 0.0,
            },
            PortfolioBet {
                combination: BetCombination::Win(horse(2)),
                method: BetMethod::Nagashi,
                stake: 300,
                odds: None,
                ev: 0.0,
                hit_prob: 0.0,
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
            method: BetMethod::Nagashi,
            stake: 100,
            odds: None,
            ev: 0.0,
            hit_prob: 0.0,
        }];
        let suggested = vec![100];
        // 1周目: 20000 を入力（budget 10000 超過）→ 再プロンプト。2周目: EOF → 0。
        let mut r = Cursor::new(b"20000\n".to_vec());
        let amounts = read_edited_amounts(&mut r, &bets, &suggested, 10000).unwrap();
        assert_eq!(amounts, vec![0]);
    }
}
