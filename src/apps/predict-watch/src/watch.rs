use std::time::Duration as StdDuration;

use chrono::{Duration, Local, NaiveTime};
use paddock_domain::{
    Portfolio, PortfolioConfig, RECOMMENDED_MARKET_BLEND_ALPHA, Race, build_portfolio,
};

use crate::cli::Cli;
use crate::setup::App;

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

/// 結果取込済み（＝発走済み）か。`races` 由来は track_condition/results が埋まると確定。
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
async fn sweep(app: &App, slots: &[Slot], now: NaiveTime, cli: &Cli, blend_alpha: Option<f64>) {
    let window = Duration::minutes(cli.window as i64);
    let due: Vec<&Slot> = slots
        .iter()
        .filter(|s| classify(now, s.post_time, has_result(&s.race), window) == RaceStatus::Due)
        .collect();
    let unknown = slots
        .iter()
        .filter(|s| classify(now, s.post_time, has_result(&s.race), window) == RaceStatus::Unknown)
        .count();

    println!(
        "── {} スイープ: 対象 {} レース（窓 {}分 / ROIゲート {:.0}%）",
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
        evaluate_race(app, slot, cli, blend_alpha).await;
    }
}

/// 1 レースを評価: フレッシュなオッズ再取得 → 確率推定 → 買い目/EV → ROI 判定。
async fn evaluate_race(app: &App, slot: &Slot, cli: &Cli, blend_alpha: Option<f64>) {
    let rid = &slot.race.race_id;
    let label = race_label(slot);

    // 1) 発走直前のフレッシュなオッズを再取得（新スナップショットを保存。read-through は使わない）。
    let odds = match app.odds.refresh_race_odds(rid).await {
        Ok(Some(o)) => o,
        Ok(None) => {
            println!("  {label}: オッズ未取得（未公開/失敗）、スキップ");
            return;
        }
        Err(e) => {
            println!("  {label}: オッズ再取得エラー: {e}");
            return;
        }
    };

    // 2) 確率推定（直前に保存したフレッシュ snapshot を内部で参照してブレンド）。
    let probs = match app
        .interactor
        .predict_race(rid, blend_alpha, slot.race.track_condition)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            println!("  {label}: 確率推定エラー: {e}");
            return;
        }
    };

    // 3) 本番と同じ軸流しポートフォリオで買い目＋EV を組成。
    let portfolio = build_portfolio(&probs, &odds, cli.race_budget, &PortfolioConfig::default());
    let Some(ev) = &portfolio.ev else {
        println!("  {label}: 買い目を組成できず（オッズ不足）、スキップ");
        return;
    };

    let roi_pct = ev.roi * 100.0;
    let hit_pct = ev.hit_prob * 100.0;
    if ev.roi >= cli.roi_gate {
        println!("  🟢 {label}: ROI {roi_pct:.1}% / 的中 {hit_pct:.1}%（張り候補）");
        print_buy_targets(&portfolio);
    } else {
        println!("  ⚪ {label}: ROI {roi_pct:.1}% / 的中 {hit_pct:.1}%（見送り）");
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
    for bet in &p.bets {
        if bet.stake == 0 {
            continue;
        }
        match bet.odds {
            Some(o) => println!(
                "     {} ¥{} オッズ{:.1} EV={:.2}",
                bet.combination.label_ja(),
                bet.stake,
                o,
                bet.ev,
            ),
            None => println!(
                "     {} ¥{} オッズ未取得",
                bet.combination.label_ja(),
                bet.stake,
            ),
        }
    }
    println!("     賭け計 ¥{}", p.total_stake);
}

/// 監視ループ。発走前のレースが残っている間スキャンを繰り返し、全レース発走で自動終了する。
pub async fn run(app: &App, cli: &Cli) -> anyhow::Result<()> {
    // α 未指定なら本番既定（market α=0.2）を使う。predict と同一値で判定を揃える。
    let blend_alpha = match cli.blend_alpha {
        Some(v) => Some(v),
        None => RECOMMENDED_MARKET_BLEND_ALPHA,
    };
    let window = Duration::minutes(cli.window as i64);

    loop {
        let slots = load_slots(app, cli.date).await?;
        let now = Local::now().time();
        sweep(app, &slots, now, cli, blend_alpha).await;

        // 終了判定: Due も NotYet も無い（＝発走前のレースが残っていない）。
        let remaining = slots.iter().any(|s| {
            matches!(
                classify(now, s.post_time, has_result(&s.race), window),
                RaceStatus::Due | RaceStatus::NotYet
            )
        });
        if !remaining {
            println!("── 監視終了: 発走前のレースが残っていません。");
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
}
