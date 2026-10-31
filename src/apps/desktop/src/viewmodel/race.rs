use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};
use paddock_use_case::interactor::race::board::{BoardHorse, FixedSelection, RaceBoard};

use crate::viewmodel::exec::{BetView, portfolio_to_bets};

#[derive(Debug, Clone, PartialEq)]
pub struct BoardView {
    pub race_id: String,
    pub date: NaiveDate,
    pub venue: String,
    pub race_num: u32,
    pub surface: String,
    pub distance: u32,
    pub post_time: Option<String>,
    pub race_name: Option<String>,
    pub race_class: Option<String>,
    pub is_confused: bool,
    pub axis_win_prob: f64,
    pub qualifying_count: u32,
    pub result_confirmed: bool,
    pub bets: Vec<BetView>,
    pub post_time_raw: Option<NaiveTime>,
    /// 買い目の軸の固定の表示（朝の固定 #767、無ければ記録◎ #388）。どちらも無ければ `None`。
    pub axis_lock: Option<AxisLockView>,
    pub horses: Vec<HorseView>,
}

/// 盤の 🔒 チップ（web の盤 `web/src/routes/RaceBoard.tsx` の固定の表示と同じ文言・分岐）。
#[derive(Debug, Clone, PartialEq)]
pub struct AxisLockView {
    /// チップの文言（先頭の 🔒 / ⚠ を含む）。
    pub label: String,
    /// 固定の理由。乖離時は展開して見せ、それ以外は title に出す。
    pub reason: &'static str,
    /// 固定した軸がライブ再計算の首位と違う（赤の ⚠ で出す）。
    pub diverged: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HorseView {
    pub horse_num: u32,
    pub horse_name: String,
    pub jockey: String,
    pub win_prob: String,
    pub pure_win_prob: String,
    pub market_implied: String,
    pub win_odds: String,
    pub model_rank: u32,
    pub mark: String,
    pub is_overlay: bool,
    pub is_value: bool,
    pub finishing_position: Option<u32>,
    pub comment: Option<String>,
}

impl From<&RaceBoard> for BoardView {
    fn from(b: &RaceBoard) -> Self {
        Self {
            race_id: b.race_id.value().to_string(),
            date: b.date,
            venue: b.venue.clone(),
            race_num: b.race_num,
            surface: b.surface.clone(),
            distance: b.distance,
            post_time: b.post_time.clone(),
            race_name: b.race_name.clone(),
            race_class: b.race_class.clone(),
            is_confused: b.confusion.is_confused,
            axis_win_prob: b.confusion.axis_win_prob,
            qualifying_count: b.confusion.qualifying_count,
            result_confirmed: b.result_confirmed,
            bets: b
                .portfolio
                .as_ref()
                .map(portfolio_to_bets)
                .unwrap_or_default(),
            post_time_raw: b
                .post_time
                .as_deref()
                .and_then(|s| NaiveTime::parse_from_str(s, "%H:%M").ok()),
            axis_lock: axis_lock(b.fixed_selection.as_ref(), b.recorded_axis, b.live_axis),
            horses: b.horses.iter().map(HorseView::from).collect(),
        }
    }
}

impl From<&BoardHorse> for HorseView {
    fn from(h: &BoardHorse) -> Self {
        Self {
            horse_num: h.horse_num,
            horse_name: h.horse_name.clone(),
            jockey: h.jockey.clone().unwrap_or_default(),
            win_prob: format!("{:.1}%", h.win_prob * 100.0),
            pure_win_prob: format!("{:.1}%", h.pure_win_prob * 100.0),
            market_implied: h
                .market_implied
                .map(|p| format!("{:.1}%", p * 100.0))
                .unwrap_or_else(|| "—".to_string()),
            win_odds: h
                .win_odds
                .map(|o| format!("{:.1}", o))
                .unwrap_or_else(|| "—".to_string()),
            model_rank: h.model_rank,
            mark: mark_symbol(h.mark.as_deref()),
            is_overlay: h.is_overlay,
            is_value: h.is_value,
            finishing_position: h.finishing_position,
            comment: h.comment.clone(),
        }
    }
}

// 以下 3 つの文言は web の盤（`web/src/routes/RaceBoard.tsx`）の写し。web 側を変えたらここも直す。
/// 記録軸の乖離時の理由（web の `AXIS_LOCK_REASON`）。
const AXIS_LOCK_REASON: &str = "買い目の軸は predict 記録済みの◎に固定しています。ライブ再計算（市場ブレンド）の首位はこれと異なりますが、軸ロック運用により差し替えません（乖離は増額判断の材料）。";
/// 朝の固定の理由（web の `MORNING_FIX_REASON`）。
const MORNING_FIX_REASON: &str = "買い目の軸・相手・混戦は、朝（監視窓に入る前）の単勝で組んだ選定に固定しています。ライブ再計算（市場ブレンド）の首位はこれと異なりますが、軸ロック運用により差し替えません（乖離は増額判断の材料）。";
/// 記録軸のチップの title（web の記録軸チップの `title`）。
const RECORDED_AXIS_TITLE: &str =
    "買い目の軸は predict 記録済みの◎に固定しています（直前オッズで軸をフリップさせない）。";

/// 盤の軸の固定の表示を組む（web の盤と同じ分岐）。朝の固定があればそのチップだけを出し
/// （記録◎ はその中で示す）、無ければ記録軸のチップ。ライブの首位（`live`）と違えば ⚠ にする。
pub fn axis_lock(
    fixed: Option<&FixedSelection>,
    recorded: Option<u32>,
    live: Option<u32>,
) -> Option<AxisLockView> {
    if let Some(f) = fixed {
        let konsen = if f.konsen_band.is_empty() {
            " 非混戦".to_string()
        } else {
            format!(" 混戦[{}]", join(&f.konsen_band))
        };
        let recorded_mark = if recorded == Some(f.axis) {
            "（記録◎）"
        } else {
            ""
        };
        // 取消で朝の軸・相手が替わったときは明示する（取消は新情報。黙って差し替えない）
        let scratched = if f.scratched.is_empty() {
            String::new()
        } else {
            format!("（取消: {}）", join(&f.scratched))
        };
        let text = format!(
            "朝 {} の選定で固定: 軸 {}{recorded_mark} 相手 {}{konsen}{scratched}",
            jst_clock(&f.odds_fetched_at),
            f.axis,
            join(&f.partners),
        );
        return Some(match live.filter(|&l| l != f.axis) {
            Some(l) => AxisLockView {
                label: format!("⚠ {text}（ライブ再計算の首位は {l}）"),
                reason: MORNING_FIX_REASON,
                diverged: true,
            },
            None => AxisLockView {
                label: format!("🔒 {text}"),
                reason: MORNING_FIX_REASON,
                diverged: false,
            },
        });
    }
    let r = recorded?;
    Some(match live.filter(|&l| l != r) {
        Some(l) => AxisLockView {
            label: format!("⚠ 記録軸 {r} 固定（ライブ再計算は {l}）"),
            reason: AXIS_LOCK_REASON,
            diverged: true,
        },
        None => AxisLockView {
            label: format!("🔒 記録軸 {r} 固定"),
            reason: RECORDED_AXIS_TITLE,
            diverged: false,
        },
    })
}

fn join(nums: &[u32]) -> String {
    nums.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// スナップショットの取得時刻（rfc3339）を JST の HH:MM にする（web の `snapshotClock` と同じく
/// JST 固定）。解釈できなければ空文字で、表示は続ける。
fn jst_clock(rfc3339: &str) -> String {
    let jst = FixedOffset::east_opt(9 * 3600).expect("+09:00 は有効なオフセット");
    DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| t.with_timezone(&jst).format("%H:%M").to_string())
        .unwrap_or_default()
}

fn mark_symbol(slug: Option<&str>) -> String {
    match slug {
        Some("honmei") => "◎",
        Some("taikou") => "○",
        Some("tanana") => "▲",
        Some("hoshi") => "☆",
        Some("renka") => "△",
        Some("chui") => "注",
        _ => "",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(
        axis: u32,
        partners: &[u32],
        konsen_band: &[u32],
        scratched: &[u32],
    ) -> FixedSelection {
        FixedSelection {
            axis,
            partners: partners.to_vec(),
            konsen_band: konsen_band.to_vec(),
            scratched: scratched.to_vec(),
            odds_fetched_at: "2026-10-10T00:31:05+00:00".to_string(),
        }
    }

    #[test]
    fn no_lock_without_fixed_or_recorded() {
        assert_eq!(axis_lock(None, None, Some(3)), None);
    }

    #[test]
    fn recorded_axis_matching_live_is_a_lock_chip() {
        let got = axis_lock(None, Some(5), Some(5)).unwrap();
        assert_eq!(got.label, "🔒 記録軸 5 固定");
        assert!(!got.diverged);
        assert_eq!(got.reason, RECORDED_AXIS_TITLE);
    }

    #[test]
    fn recorded_axis_without_live_is_not_diverged() {
        let got = axis_lock(None, Some(5), None).unwrap();
        assert_eq!(got.label, "🔒 記録軸 5 固定");
        assert!(!got.diverged);
    }

    #[test]
    fn recorded_axis_diverging_from_live_warns() {
        let got = axis_lock(None, Some(5), Some(7)).unwrap();
        assert_eq!(got.label, "⚠ 記録軸 5 固定（ライブ再計算は 7）");
        assert!(got.diverged);
        assert_eq!(got.reason, AXIS_LOCK_REASON);
    }

    #[test]
    fn morning_fix_shows_jst_clock_axis_partners_and_konsen() {
        let f = fixed(11, &[1, 2, 3], &[1, 2, 3, 11], &[]);
        let got = axis_lock(Some(&f), None, Some(11)).unwrap();
        assert_eq!(
            got.label,
            "🔒 朝 09:31 の選定で固定: 軸 11 相手 1,2,3 混戦[1,2,3,11]"
        );
        assert!(!got.diverged);
        assert_eq!(got.reason, MORNING_FIX_REASON);
    }

    #[test]
    fn morning_fix_not_konsen_with_recorded_axis_and_scratches() {
        let f = fixed(4, &[2, 6], &[], &[9]);
        let got = axis_lock(Some(&f), Some(4), None).unwrap();
        assert_eq!(
            got.label,
            "🔒 朝 09:31 の選定で固定: 軸 4（記録◎） 相手 2,6 非混戦（取消: 9）"
        );
        assert!(!got.diverged);
    }

    #[test]
    fn morning_fix_takes_precedence_over_a_different_recorded_axis() {
        let f = fixed(4, &[2, 6], &[], &[]);
        let got = axis_lock(Some(&f), Some(8), Some(4)).unwrap();
        assert_eq!(got.label, "🔒 朝 09:31 の選定で固定: 軸 4 相手 2,6 非混戦");
        assert_eq!(got.reason, MORNING_FIX_REASON);
        assert!(!got.diverged);
    }

    #[test]
    fn fractional_seconds_in_fetched_at_still_show_the_jst_clock() {
        let mut f = fixed(4, &[2], &[], &[]);
        f.odds_fetched_at = "2026-10-10T00:31:05.123+00:00".to_string();
        let got = axis_lock(Some(&f), None, None).unwrap();
        assert_eq!(got.label, "🔒 朝 09:31 の選定で固定: 軸 4 相手 2 非混戦");
    }

    #[test]
    fn morning_fix_diverging_from_live_warns() {
        let f = fixed(4, &[2, 6], &[], &[]);
        let got = axis_lock(Some(&f), None, Some(2)).unwrap();
        assert_eq!(
            got.label,
            "⚠ 朝 09:31 の選定で固定: 軸 4 相手 2,6 非混戦（ライブ再計算の首位は 2）"
        );
        assert!(got.diverged);
        assert_eq!(got.reason, MORNING_FIX_REASON);
    }

    #[test]
    fn unparsable_fetched_at_leaves_the_clock_empty() {
        let mut f = fixed(4, &[2], &[], &[]);
        f.odds_fetched_at = "not-a-time".to_string();
        let got = axis_lock(Some(&f), None, None).unwrap();
        assert_eq!(got.label, "🔒 朝  の選定で固定: 軸 4 相手 2 非混戦");
    }
}
