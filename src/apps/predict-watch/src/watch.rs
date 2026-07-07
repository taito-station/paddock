use std::time::Duration as StdDuration;

use chrono::{Duration, Local, NaiveTime, Offset, Utc};
use paddock_domain::{
    BetMethod, Portfolio, PortfolioConfig, RECOMMENDED_MARKET_BLEND_ALPHA, Race, build_portfolio,
};
use paddock_use_case::PredictionViews;
use predict_format::{format_explanations, format_probs, format_probs_with_market};

use crate::cli::Cli;
use crate::setup::App;
use crate::snapshot::{SnapshotContext, build_snapshot_record};

/// `now` 時点でのレースの発走状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceStatus {
    /// 発走前で先読み窓内 → オッズ再取得＆EV 再計算の対象。
    Due,
    /// 発走前だが窓より先 → まだ対象外（次スイープ以降に Due 化）。
    NotYet,
    /// 発走済み（結果取込済み or 発走時刻超過）→ 対象外。
    Started,
    /// 発走時刻不明（post_time 無し）→ 判定不能、対象外。
    Unknown,
}

/// `now` 時点でのレース発走状態を判定する純関数（単体テスト対象）。
///
/// - post_time 無し → `Unknown`
/// - 結果取込済み（`has_result`）または発走時刻超過（`now > post`）→ `Started`
/// - 発走前で残り時間が `window` 以内 → `Due`、それより先 → `NotYet`
pub fn classify(
    now: NaiveTime,
    post_time: Option<NaiveTime>,
    has_result: bool,
    window: Duration,
) -> RaceStatus {
    let Some(post) = post_time else {
        return RaceStatus::Unknown;
    };
    if has_result || now > post {
        return RaceStatus::Started;
    }
    // ここで now <= post。発走まで (post - now)。窓内なら Due。
    if post - now <= window {
        RaceStatus::Due
    } else {
        RaceStatus::NotYet
    }
}

/// 監視を継続すべきか（発走前のレースが残っているか）を判定する純関数（単体テスト対象）。
/// Due か NotYet が 1 つでもあれば継続、無ければ終了。
pub fn should_continue(statuses: &[RaceStatus]) -> bool {
    statuses
        .iter()
        .any(|s| matches!(s, RaceStatus::Due | RaceStatus::NotYet))
}

/// 結果取込済み（＝確実に発走済み）か。`races_by_date` は発走前レースを race_cards 由来で
/// track_condition=NULL・results 空として返す（fact-check 済みの不変条件）ため、この経路では
/// track_condition/results は「成績取込が済んだ＝確実に過去のレース」を指す早期シグナル。
/// 成績取込前でも発走済みになる通常遷移（発走直後）は classify の `now > post` 側が捕捉する。
fn has_result(race: &Race) -> bool {
    race.track_condition.is_some() || !race.results.is_empty()
}

/// 発走時刻付きのレース 1 件（races_by_date の Race ＋ race_card の post_time）。
struct Slot {
    race: Race,
    post_time: Option<NaiveTime>,
}

/// 指定日の全レースを取得し、各レースの post_time（race_card 由来）を引き当てる。
async fn load_slots(app: &App, date: chrono::NaiveDate) -> anyhow::Result<Vec<Slot>> {
    let races = app.interactor.races_by_date(date).await?;
    let mut slots = Vec::with_capacity(races.len());
    for race in races {
        let post_time = app
            .interactor
            .race_card(&race.race_id)
            .await?
            .and_then(|c| c.post_time);
        slots.push(Slot { race, post_time });
    }
    Ok(slots)
}

/// レース表示ラベル（例: `函館10R 15:35`）。発走時刻不明なら `--:--`。
fn race_label(slot: &Slot) -> String {
    let post = slot
        .post_time
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_else(|| "--:--".to_string());
    format!(
        "{}{}R {}",
        slot.race.venue.as_jp(),
        slot.race.race_num,
        post
    )
}

/// 1 スイープ: Due レースのオッズを再取得し EV/ROI を再計算、結果を 1 行ずつ出力する。
/// `statuses` は `slots` と同順の発走状態（呼び出し側で 1 度だけ算出して使い回す）。
async fn sweep(
    app: &App,
    slots: &[Slot],
    statuses: &[RaceStatus],
    now: NaiveTime,
    captured_at: &str,
    cli: &Cli,
    blend_alpha: Option<f64>,
) {
    let due: Vec<&Slot> = slots
        .iter()
        .zip(statuses)
        .filter(|(_, st)| **st == RaceStatus::Due)
        .map(|(s, _)| s)
        .collect();
    let unknown = statuses
        .iter()
        .filter(|st| **st == RaceStatus::Unknown)
        .count();

    println!(
        "── {} スイープ: 対象 {} レース（窓 {}分 / 参考ROIゲート {:.0}%・判定は手動精査）",
        now.format("%H:%M"),
        due.len(),
        cli.window,
        cli.roi_gate * 100.0,
    );
    if unknown > 0 {
        println!(
            "   ※ 発走時刻不明（post_time 無し）{unknown} レースは対象外。fetch-card 済みか確認。"
        );
    }
    for slot in due {
        evaluate_race(app, slot, captured_at, cli, blend_alpha).await;
    }
}

/// 1 レースを評価: フレッシュなオッズ再取得 → 確率推定 → 買い目/EV → ROI 判定。
async fn evaluate_race(
    app: &App,
    slot: &Slot,
    captured_at: &str,
    cli: &Cli,
    blend_alpha: Option<f64>,
) {
    let rid = &slot.race.race_id;
    let label = race_label(slot);

    // 1) 発走直前のフレッシュなオッズを再取得（新スナップショットを保存。read-through は使わない）。
    let odds = match app.odds.refresh_race_odds(rid).await {
        Ok(Some(o)) => o,
        Ok(None) => {
            println!("  {label}: オッズ未取得（未公開/失敗）、スキップ");
            return;
        }
        // refresh_race_odds は現状スクレイプ失敗を Ok(None) に畳むため Err は来ないが、
        // 将来の戻り値変更（DB エラー伝播等）に備えて防御的に握っておく。
        Err(e) => {
            println!("  {label}: オッズ再取得エラー: {e}");
            return;
        }
    };

    // 2) 確率を 2 視点で推定（#272 確率分離）。predict_race_views は factor 収集 1 回で blended（順位
    //    付け・市場ブレンド）と pure（EV 用・純モデル α=1.0）を返す。内部で find_race_odds（直前に
    //    persist した最新スナップショット）を再読込して blended のブレンドに使う。build_portfolio へ渡す
    //    odds と同一データだが読み出し経路は別。persist 失敗時（warn のみで継続）は旧スナップショットを
    //    見るため、その回だけ買い目側と確率側でオッズ集合が食い違いうる（次スイープで解消する一時的劣化）。
    //    track_condition は発走前レースでは None のため、監視は当日の馬場状態を反映しない。
    let PredictionViews {
        blended,
        pure,
        explanations,
    } = match app
        .interactor
        .predict_race_views(rid, blend_alpha, slot.race.track_condition, true)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            println!("  {label}: 確率推定エラー: {e}");
            return;
        }
    };

    // 過去データ視点（純モデルの順位＋根拠）。EV に依らず常に出す。エッジが無い窓でも「なぜこの順位か」を提示。
    println!("  ── {label} 過去データ視点（純モデル）");
    for line in format_probs(&pure) {
        println!("    {line}");
    }
    for line in format_explanations(&pure, &explanations) {
        println!("    {line}");
    }
    // 純モデル vs 市場implied（差で割安/割高の向きを読む）。
    let market_win: std::collections::HashMap<_, _> =
        odds.win.iter().map(|(num, o)| (*num, o.value())).collect();
    for line in format_probs_with_market(&pure, &market_win) {
        println!("    {line}");
    }

    // 3) 市場EV視点: 軸/相手は blended、EV/的中は pure（循環断ち, #272）。
    let portfolio = build_portfolio(
        &blended,
        &pure,
        &odds,
        cli.race_budget,
        &PortfolioConfig::default(),
    );
    let Some(ev) = &portfolio.ev else {
        println!("  {label}: 買い目を組成できず（オッズ不足）、スキップ");
        return;
    };

    // decision-support（#272）: 自動の張る/見送り判定はしない。純モデル EV/ROI は「市場に対しモデルが
    // 割安と見るか」の参考情報で、最終判断は人間のハンデ精査に委ねる。参考 ROI がゲート以上のときだけ
    // 🔶 を付けて目立たせる（張り推奨ではない）。
    let roi_pct = ev.roi * 100.0;
    let hit_pct = ev.hit_prob * 100.0;
    let mark = if ev.roi >= cli.roi_gate {
        "🔶"
    } else {
        "・"
    };
    println!(
        "  {mark} {label}: 参考ROI {roi_pct:.1}% / 的中 {hit_pct:.1}%（モデル単独視点・最終判断は手動精査）"
    );
    print_buy_targets(&portfolio);

    // 4) ライブ EV ビュー/アーカイブ（live_ev_snapshots）へ best-effort で永続化（#346 / ADR 0064）。
    //    ここは decision-support のスナップショット記録であり、predict のセッション記録
    //    （predict_sessions / predict_bets）には触れない。保存失敗で監視ループは止めない。
    if let Some(axis) = portfolio.axis {
        // ◎の model 勝率[%]は blended（順位付け視点）の軸馬から採る。axis は build_portfolio が
        // blended の勝率最上位から選ぶため必ず blended に含まれる。unwrap_or(0.0) は到達しない前提の
        // 防御で、万一の不整合でも監視を止めず 0% として記録する（無言のクラッシュより望ましい）。
        let axis_prob = blended
            .iter()
            .find(|hp| hp.horse_num == axis)
            .map(|hp| hp.win_prob * 100.0)
            .unwrap_or(0.0);
        let axis_win_odds = odds.win.get(&axis).map(|o| o.value());
        let (axis_place_low, axis_place_high) = odds
            .place
            .get(&axis)
            .map(|b| (Some(b.low.value()), Some(b.high.value())))
            .unwrap_or((None, None));
        let post_time = slot.post_time.map(|t| t.format("%H:%M").to_string());

        let ctx = SnapshotContext {
            date: cli.date,
            race_id: rid.value(),
            venue: slot.race.venue.as_slug(),
            race_no: slot.race.race_num,
            post_time,
            captured_at,
            axis_prob,
            axis_win_odds,
            axis_place_odds_low: axis_place_low,
            axis_place_odds_high: axis_place_high,
            race_budget: cli.race_budget,
        };
        if let Some(record) = build_snapshot_record(&portfolio, &ctx)
            && let Err(e) = app.interactor.save_live_ev_snapshot(&record).await
        {
            println!("  {label}: ライブEVスナップショット保存に失敗（監視は継続）: {e}");
        }
    }
}

/// 張り候補の買い目を「そのまま買える形」で出力する（軸/相手＋各点）。
fn print_buy_targets(p: &Portfolio) {
    if let Some(axis) = p.axis {
        let rel = p
            .partners
            .iter()
            .map(|h| h.value().to_string())
            .collect::<Vec<_>>()
            .join(",");
        println!("     軸 {} → 相手 {}", axis.value(), rel);
    }
    if p.bets.iter().any(|b| b.method == BetMethod::Box) {
        println!("     混戦: 印馬3連複ボックス（軸なし）を併用");
    }
    for bet in &p.bets {
        if bet.stake == 0 {
            continue;
        }
        // 方式（ながし/ボックス）を明示。box は軸なしの印馬総当たりで「軸流し」枠と区別する（表記規約）。
        let method = match bet.method {
            BetMethod::Nagashi => "ながし",
            BetMethod::Box => "ボックス",
        };
        match bet.odds {
            Some(o) => println!(
                "     [{}] {} ¥{} オッズ{:.1} 的中{:.1}% EV={:.2}",
                method,
                bet.combination.label_ja(),
                bet.stake,
                o,
                bet.hit_prob * 100.0,
                bet.ev,
            ),
            None => println!(
                "     [{}] {} ¥{} オッズ未取得 的中{:.1}%",
                method,
                bet.combination.label_ja(),
                bet.stake,
                bet.hit_prob * 100.0,
            ),
        }
    }
    println!("     賭け計 ¥{}", p.total_stake);
}

/// 監視ループ。発走前のレースが残っている間スキャンを繰り返し、全レース発走で自動終了する。
pub async fn run(app: &App, cli: &Cli) -> anyhow::Result<()> {
    // 連続再取得（busy loop）で JRA を叩き続けないよう、間隔・窓は 1 分以上を要求する。
    if cli.interval == 0 {
        anyhow::bail!(
            "--interval は 1 分以上を指定してください（0 は JRA への連続再取得になり礼節に反します）。"
        );
    }
    if cli.window == 0 {
        anyhow::bail!("--window は 1 分以上を指定してください。");
    }
    // α は市場とモデルのブレンド重み（0=市場のみ, 1=モデルのみ）。範囲外は無意味なので弾く。
    if let Some(a) = cli.blend_alpha
        && !(0.0..=1.0).contains(&a)
    {
        anyhow::bail!(
            "--blend-alpha は 0.0〜1.0 で指定してください（市場とモデルのブレンド重み）。"
        );
    }

    // α 未指定なら本番既定（market α=0.2）を使う。predict と同一値で判定を揃える。
    let blend_alpha = cli.blend_alpha.or(RECOMMENDED_MARKET_BLEND_ALPHA);
    let window = Duration::minutes(cli.window as i64);

    // 発走状態は実行マシンの現在時刻と post_time の「時刻」だけで判定するため、(1) 当日以外の date、
    // (2) JST 以外の TZ では判定が無意味になる。誤用に早期に気づけるよう起動時に警告する。
    const JST_OFFSET_SECS: i32 = 9 * 3600;
    let now_local = Local::now();
    let today = now_local.date_naive();
    if cli.date != today {
        println!(
            "⚠ --date {} は本日（{today}）と異なります。発走状態は現在時刻と post_time の時刻のみで \
             判定するため、当日以外の指定では Due/Started 判定が正しく機能しません。",
            cli.date,
        );
    }
    let tz_offset = now_local.offset().fix().local_minus_utc();
    if tz_offset != JST_OFFSET_SECS {
        // 半端な TZ（例 +05:30）も正しく出せるよう ±HH:MM 表記にする。
        let sign = if tz_offset < 0 { '-' } else { '+' };
        let abs = tz_offset.abs();
        println!(
            "⚠ 実行マシンのタイムゾーンが JST(+09:00) ではありません（現在 UTC{sign}{:02}:{:02}）。\
             post_time は JST 起算のため、発走状態判定がオフセットぶんずれます。JST マシンで実行してください。",
            abs / 3600,
            (abs % 3600) / 60,
        );
    }

    loop {
        // 継続監視中の一時的 DB エラーでプロセスを落とすと「唯一エッジのある局面」を取りこぼす。
        // evaluate_race と同じく握って次スイープへ続行する（--once 時のみ伝播して非ゼロ終了）。
        let slots = match load_slots(app, cli.date).await {
            Ok(s) => s,
            Err(e) if cli.once => return Err(e),
            Err(e) => {
                println!("⚠ レース一覧の取得に失敗（次スイープで再試行）: {e}");
                tokio::time::sleep(StdDuration::from_secs(cli.interval * 60)).await;
                continue;
            }
        };
        let now = Local::now().time();
        // 監視サイクル境界時刻（1 スイープ 1 値・UTC rfc3339 秒精度 Z 終端）。同一サイクルの全レースが
        // 同一 captured_at を共有し、live_ev_snapshots の「辞書順＝時刻順」「(race_id, captured_at) 冪等」
        // 契約を満たす（旧 refresh_ev.sh の `date -u +%Y-%m-%dT%H:%M:%SZ` と同表記）。
        let captured_at = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        // 発走状態は 1 スイープ 1 回だけ算出し、sweep 表示と終了判定で共有する。
        let statuses: Vec<RaceStatus> = slots
            .iter()
            .map(|s| classify(now, s.post_time, has_result(&s.race), window))
            .collect();

        // 防御: 発走前（now <= post）なのに結果取込済みのレースは、races_by_date の不変条件
        //（発走前＝race_cards 由来で track_condition=NULL）が崩れた兆候。放置すると Started 誤判定で
        // 監視が無言の no-op 化するため、検出したら警告して気づけるようにする。
        let started_before_post = slots
            .iter()
            .filter(|s| has_result(&s.race) && s.post_time.is_some_and(|p| now <= p))
            .count();
        if started_before_post > 0 {
            println!(
                "⚠ 発走前なのに結果取込済みのレースが {started_before_post} 件あります。発走状態判定の前提が \
                 崩れている可能性があり、対象から外れます（fetch-card / 成績取込の状態を確認してください）。"
            );
        }

        sweep(app, &slots, &statuses, now, &captured_at, cli, blend_alpha).await;

        if !should_continue(&statuses) {
            if statuses.is_empty() {
                println!("── 監視終了: 本日（{}）は対象開催がありません。", cli.date);
            } else if statuses.iter().all(|s| *s == RaceStatus::Unknown) {
                println!(
                    "── 監視終了: 全レースで発走時刻（post_time）が不明です。fetch-card 済みか確認してください。"
                );
            } else {
                println!("── 監視終了: 発走前のレースが残っていません。");
            }
            break;
        }
        if cli.once {
            println!("── --once 指定のため 1 スイープで終了します。");
            break;
        }
        tokio::time::sleep(StdDuration::from_secs(cli.interval * 60)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn unknown_when_no_post_time() {
        assert_eq!(
            classify(t(15, 0), None, false, Duration::minutes(40)),
            RaceStatus::Unknown
        );
    }

    #[test]
    fn started_when_result_present() {
        // 結果取込済みは発走前の時刻でも Started（再取得しない）。
        assert_eq!(
            classify(t(14, 0), Some(t(15, 0)), true, Duration::minutes(40)),
            RaceStatus::Started
        );
    }

    #[test]
    fn started_when_now_past_post() {
        assert_eq!(
            classify(t(15, 1), Some(t(15, 0)), false, Duration::minutes(40)),
            RaceStatus::Started
        );
    }

    #[test]
    fn due_within_window_inclusive_boundary() {
        // 残り 40 分ちょうどは窓内（境界を含む）。
        assert_eq!(
            classify(t(14, 20), Some(t(15, 0)), false, Duration::minutes(40)),
            RaceStatus::Due
        );
        // 残り 1 分も Due。
        assert_eq!(
            classify(t(14, 59), Some(t(15, 0)), false, Duration::minutes(40)),
            RaceStatus::Due
        );
        // 発走時刻ちょうど（残り 0 分）も発走前扱いで Due。
        assert_eq!(
            classify(t(15, 0), Some(t(15, 0)), false, Duration::minutes(40)),
            RaceStatus::Due
        );
    }

    #[test]
    fn not_yet_when_outside_window() {
        // 残り 41 分は窓の外。
        assert_eq!(
            classify(t(14, 19), Some(t(15, 0)), false, Duration::minutes(40)),
            RaceStatus::NotYet
        );
    }

    #[test]
    fn should_continue_while_due_or_not_yet_remains() {
        use RaceStatus::*;
        // Due か NotYet が 1 つでもあれば継続。
        assert!(should_continue(&[Started, Due, Started]));
        assert!(should_continue(&[Started, NotYet]));
        // 全て発走済み or 不明なら終了。
        assert!(!should_continue(&[Started, Started, Unknown]));
        assert!(!should_continue(&[Unknown]));
        // 空（その日に開催なし）も終了。
        assert!(!should_continue(&[]));
    }
}
