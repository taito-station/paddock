//! 朝の選定（軸・相手・混戦）で買い目を固定する（#767・REQ-D01-003・REQ-D23-007）。
//!
//! 朝 = 当日 JST 0 時以降で最初に単勝を取ったスナップショット（#724 と同じ定義。#448 の盤の「朝↔現」
//! ＝最初に全券種がそろった時点とは別物）。その盤で本番と同じ [`compose_portfolio`] で組み、伝票から
//! 記録側と同じ規則（[`PinnedSelection::from_staked_legs`]）で選定を読む。
//!
//! 監視窓に入った後に固定の元を入れ替えない（軸フリップを起こさない）ため、次の 2 つを守る:
//! - 朝として採るのは、監視窓に入る時刻より前に保存されたスナップショットだけ（predict-watch 自身の
//!   初回スイープの保存を「朝」と取り違えない）。
//! - 初回スイープ（T-40）で既に固定したレースは、後から朝の選定に替えない（[`resolve_fixed_selection`]）。

use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::{
    EstimationConfig, HorseNum, PinnedSelection, RECOMMENDED_MARKET_BLEND_ALPHA, RaceId, RaceOdds,
};

use super::orchestrate::{PredictionViews, compose_portfolio};
use crate::error::{Error, Result};
use crate::interactor::Interactor;
use crate::interactor::race::board::recorded_axis_of;
use crate::repository::{
    LiveEvPin, MorningSelection, MorningSelectionRepository, OddsRepository,
    PadPredictionRepository, RaceCardRepository, SnapshotPoint, StatsRepository,
};

/// 朝の選定を組むときの予算（CLAUDE.md「予算・配分」の既定 ¥5,000/レース）。
///
/// `from_staked_legs` は賭金の乗った脚だけを読むので、予算が小さいと相手が減る。起動時の
/// `--race-budget` や per-race の増額に選定が左右されないよう、本番の既定で固定する。
pub const MORNING_SELECTION_BUDGET: u64 = 5000;

/// 開催日 `date` の JST 0 時を UTC で表す（朝のスナップショットの起点）。
pub(crate) fn jst_midnight_utc(date: NaiveDate) -> DateTime<Utc> {
    let midnight = date.and_hms_opt(0, 0, 0).expect("0 時は常に有効");
    DateTime::<Utc>::from_naive_utc_and_offset(midnight, Utc) - chrono::Duration::hours(9)
}

/// 推定・オッズから選定を読む。`recorded_axis`（記録◎）があれば軸に強制して組む
/// （軸だけ後から差し替えると、相手が 4 頭に減りモデルの首位が買い目から消えるため）。
pub fn pick_selection(
    views: &PredictionViews,
    odds: &RaceOdds,
    recorded_axis: Option<HorseNum>,
    budget: u64,
) -> PinnedSelection {
    let portfolio = compose_portfolio(
        views,
        odds,
        budget,
        &PinnedSelection::axis_only(recorded_axis),
    );
    PinnedSelection::from_staked_legs(&portfolio)
}

/// [`Interactor::ensure_morning_selection`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MorningOutcome {
    /// 朝の選定がある（既に保存済み、またはこの呼び出しで保存した。値は DB から読み直したもの）。
    Ready(MorningSelection),
    /// 当日 JST 0 時以降の単勝スナップショットがまだ無い（次のスイープで計算し直す）。
    NoSnapshot,
    /// 最初の単勝スナップショットが監視窓に入った後のもの（朝の選定は作らない。T-40 で固定する）。
    TooLate { odds_fetched_at: String },
    /// 朝の盤で買い目を組めなかった（出走馬が足りない等）。
    NotComposable,
}

/// 固定の出所（ログ・盤の表示用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixSource {
    /// 固定なし（このスイープで選定が決まる）。
    None,
    /// 記録◎ の軸だけ。
    RecordedAxis,
    /// 朝の選定（`odds_fetched_at` は朝のスナップショットの時刻）。`recorded` は軸が記録◎ か。
    Morning {
        odds_fetched_at: String,
        recorded: bool,
    },
    /// その日の初回スイープ（T-40）。`recorded` は軸が記録◎ か。
    FirstSweep { captured_at: String, recorded: bool },
}

/// 固定の出所を表示用の文言にする（predict-watch のログと対話の predict の 🔒 行で共有・#776）。
/// 例: `朝 09:12` / `記録◎ ＋ 初回スイープ 15:00` / `記録◎` / `固定なし`。時刻は JST の `HH:MM`。
pub fn fix_source_label(source: &FixSource) -> String {
    let with_recorded = |recorded: bool, base: String| {
        if recorded {
            format!("記録◎ ＋ {base}")
        } else {
            base
        }
    };
    match source {
        FixSource::None => "固定なし".to_string(),
        FixSource::RecordedAxis => "記録◎".to_string(),
        FixSource::Morning {
            odds_fetched_at,
            recorded,
        } => with_recorded(*recorded, format!("朝 {}", jst_hhmm(odds_fetched_at))),
        FixSource::FirstSweep {
            captured_at,
            recorded,
        } => with_recorded(*recorded, format!("初回スイープ {}", jst_hhmm(captured_at))),
    }
}

/// UTC rfc3339 を JST `HH:MM` にする。解釈できなければそのまま返す（表示を止めず、人が気づけるように）。
fn jst_hhmm(rfc3339: &str) -> String {
    DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .and_then(|dt| {
            chrono::FixedOffset::east_opt(9 * 3600)
                .map(|jst| dt.with_timezone(&jst).format("%H:%M").to_string())
        })
        .unwrap_or_else(|| rfc3339.to_string())
}

fn parse_utc(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn to_nums(v: &[u32]) -> Vec<HorseNum> {
    v.iter()
        .filter_map(|n| HorseNum::try_from(*n).ok())
        .collect()
}

/// レース 1 件について何を固定するかを決める（#767。#601 の優先順に朝を足したもの）。
///
/// 優先順:
/// 1. **朝の選定**。軸・相手・混戦をまとめて固定する。記録◎（pad の Honmei）は朝の選定を組むときに
///    軸へ強制済みなので、ここでは使わない——**保存した後に入った記録◎ は使わない**（盤と predict-watch で
///    固定をそろえ、相手が欠ける「軸だけ後から差し替え」を作らない。#767 の review でユーザー判断）。
/// 2. **その日の初回スイープ（T-40）**。朝の選定が無いレースのフォールバック。記録◎ があれば軸だけ上書きする
///    （#601 からの規則。predict-watch は初回スイープの固定がある間は記録◎ を読まない）。
/// 3. どちらも無ければ記録◎ の軸だけ、それも無ければ固定なし。
///
/// 朝の選定と初回スイープが両方あり、**朝の選定のほうが後に計算された**ときは初回スイープを採る
/// （窓に入った後に固定の元を入れ替えると軸フリップになる。REQ-D01-003）。時刻を解釈できなければ
/// 安全側（初回スイープ）に倒す。盤は初回スイープを持たないので `first_sweep` に `None` を渡す。
pub fn resolve_fixed_selection(
    recorded_axis: Option<HorseNum>,
    morning: Option<&MorningSelection>,
    first_sweep: Option<&LiveEvPin>,
) -> (PinnedSelection, FixSource) {
    let recorded = recorded_axis.is_some();
    let morning = morning.filter(|m| match first_sweep {
        None => true,
        Some(p) => matches!(
            (parse_utc(&m.computed_at), parse_utc(&p.captured_at)),
            (Some(c), Some(f)) if c <= f
        ),
    });
    let (axis, partners, band, source) = match (morning, first_sweep) {
        (Some(m), _) => (
            HorseNum::try_from(m.axis).ok(),
            &m.partners,
            &m.konsen_band,
            FixSource::Morning {
                odds_fetched_at: m.odds_fetched_at.clone(),
                // 朝の選定を組んだときに記録◎ を強制していれば、軸は記録◎ と一致する
                recorded: recorded_axis.is_some_and(|r| r.value() == m.axis),
            },
        ),
        (None, Some(p)) => (
            recorded_axis.or_else(|| HorseNum::try_from(p.axis).ok()),
            &p.partners,
            &p.konsen_band,
            FixSource::FirstSweep {
                captured_at: p.captured_at.clone(),
                recorded,
            },
        ),
        (None, None) => {
            let source = if recorded {
                FixSource::RecordedAxis
            } else {
                FixSource::None
            };
            return (PinnedSelection::axis_only(recorded_axis), source);
        }
    };
    (
        PinnedSelection {
            axis,
            partners: Some(to_nums(partners)),
            konsen_band: Some(to_nums(band)),
        },
        source,
    )
}

impl<
    R: StatsRepository
        + RaceCardRepository
        + OddsRepository
        + MorningSelectionRepository
        + PadPredictionRepository,
> Interactor<R>
{
    /// 1 レースの朝の選定を返す。まだ無ければ朝のスナップショットから計算して保存する（#767）。
    ///
    /// - `window_entry`: そのレースが監視窓に入る時刻。これ以降に保存された単勝は「朝」と見なさない。
    /// - 記録◎（pad の Honmei）が出走していれば、軸に強制して組む（組むと決まってから引く）。
    /// - `computed_at`: 記録する計算時刻（UTC rfc3339）。predict-watch はスイープの `captured_at` を
    ///   渡す。初回スイープとの前後は [`resolve_fixed_selection`] がこの値で比べる。
    ///
    /// 推定は本番の設定（[`EstimationConfig::production`]・[`RECOMMENDED_MARKET_BLEND_ALPHA`]・
    /// [`MORNING_SELECTION_BUDGET`]）で行い、起動時の引数には左右されない。保存は先勝ちで、
    /// 保存の後に読み直した DB の値を返す（二重起動でも先に保存された選定にそろう）。
    pub async fn ensure_morning_selection(
        &self,
        race_id: &RaceId,
        date: NaiveDate,
        window_entry: DateTime<Utc>,
        computed_at: &str,
    ) -> Result<MorningOutcome> {
        if let Some(saved) = self
            .repository
            .find_morning_selection(race_id.value())
            .await?
        {
            // 延期・代替開催では race_id はそのまま日付だけ変わる。前の日の朝の選定は使わない
            // （race_id が主キーなので新しい日の選定も保存できない。初回スイープの固定に戻る）
            if saved.date != date {
                return Err(Error::Internal(format!(
                    "{} の朝の選定は別の日（{}）のもの。使わずに初回スイープの固定で続ける",
                    race_id.value(),
                    saved.date
                )));
            }
            return Ok(MorningOutcome::Ready(saved));
        }
        let Some(snapshot) = self
            .repository
            .find_race_odds_snapshot(
                race_id,
                SnapshotPoint::FirstWinSince(jst_midnight_utc(date)),
            )
            .await?
        else {
            return Ok(MorningOutcome::NoSnapshot);
        };
        // 時刻を解釈できないスナップショットは朝と見なさない（安全側）。
        if parse_utc(&snapshot.fetched_at).is_none_or(|t| t >= window_entry) {
            return Ok(MorningOutcome::TooLate {
                odds_fetched_at: snapshot.fetched_at,
            });
        }
        let views = self
            .predict_race_views_at(
                race_id,
                RECOMMENDED_MARKET_BLEND_ALPHA,
                &snapshot.odds,
                date,
                &EstimationConfig::production(),
            )
            .await?;
        // 記録◎ は組むと決まってから引く（朝の単勝待ちのレースで毎スイープ引かない）
        let pad = match self.race_card(race_id).await? {
            Some(card) => {
                self.repository
                    .find_pad_prediction(card.date, card.venue, card.race_num)
                    .await?
            }
            None => None,
        };
        let recorded =
            recorded_axis_of(pad.as_ref(), &views.blended).and_then(|n| HorseNum::try_from(n).ok());
        let pick = pick_selection(&views, &snapshot.odds, recorded, MORNING_SELECTION_BUDGET);
        let Some(axis) = pick.axis else {
            return Ok(MorningOutcome::NotComposable);
        };
        let values = |v: Option<Vec<HorseNum>>| -> Vec<u32> {
            let mut out: Vec<u32> = v.unwrap_or_default().iter().map(|n| n.value()).collect();
            out.sort_unstable();
            out
        };
        let selection = MorningSelection {
            race_id: race_id.value().to_string(),
            date,
            axis: axis.value(),
            partners: values(pick.partners),
            konsen_band: values(pick.konsen_band),
            odds_fetched_at: snapshot.fetched_at,
            computed_at: computed_at.to_string(),
        };
        self.repository.save_morning_selection(&selection).await?;
        match self
            .repository
            .find_morning_selection(race_id.value())
            .await?
        {
            Some(saved) => Ok(MorningOutcome::Ready(saved)),
            None => Err(Error::Internal(format!(
                "朝の選定を保存した直後に読み出せない: {}",
                race_id.value()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use paddock_domain::{BetType, HorseName, HorseProbability, OddsValue};

    use super::*;
    use crate::interactor::race::predict::RecentRunsCoverage;

    #[test]
    fn jst_hhmm_converts_utc_and_passes_through_garbage() {
        assert_eq!(jst_hhmm("2026-08-09T00:15:03Z"), "09:15");
        assert_eq!(jst_hhmm("2026-08-09T09:27:37+00:00"), "18:27");
        // 解釈できない値でも表示を止めず、そのまま出して人が気づけるようにする。
        assert_eq!(jst_hhmm("not-a-time"), "not-a-time");
    }

    fn hn(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    fn morning(axis: u32, computed_at: &str) -> MorningSelection {
        MorningSelection {
            race_id: "2026-4-tokyo-4-11R".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 10, 10).unwrap(),
            axis,
            partners: vec![1, 2, 3, 4, 5],
            konsen_band: vec![],
            odds_fetched_at: "2026-10-10T00:31:05.1+00:00".to_string(),
            computed_at: computed_at.to_string(),
        }
    }

    fn first_sweep(axis: u32, captured_at: &str) -> LiveEvPin {
        LiveEvPin {
            race_id: "2026-4-tokyo-4-11R".to_string(),
            axis,
            partners: vec![6, 7, 8, 9, 10],
            konsen_band: vec![6, 7, 8, 9],
            captured_at: captured_at.to_string(),
        }
    }

    const EARLY: &str = "2026-10-10T01:00:00Z";
    const LATE: &str = "2026-10-10T05:00:00Z";

    #[test]
    fn nothing_to_fix_is_empty() {
        let (p, s) = resolve_fixed_selection(None, None, None);
        assert!(p.is_empty());
        assert_eq!(s, FixSource::None);
    }

    #[test]
    fn recorded_axis_alone_pins_axis_only() {
        let (p, s) = resolve_fixed_selection(Some(hn(8)), None, None);
        assert_eq!(p.axis, Some(hn(8)));
        assert_eq!((p.partners, p.konsen_band), (None, None), "軸だけ");
        assert_eq!(s, FixSource::RecordedAxis);
    }

    #[test]
    fn morning_pins_all_three() {
        let (p, s) = resolve_fixed_selection(None, Some(&morning(11, EARLY)), None);
        assert_eq!(p.axis, Some(hn(11)));
        assert_eq!(p.partners, Some(vec![hn(1), hn(2), hn(3), hn(4), hn(5)]));
        assert_eq!(p.konsen_band, Some(vec![]), "空は非混戦で固定");
        assert!(matches!(
            s,
            FixSource::Morning {
                recorded: false,
                ..
            }
        ));
    }

    #[test]
    fn morning_wins_over_a_later_first_sweep() {
        // 朝の選定（窓に入る前に計算）があれば、その後の初回スイープより優先する
        let (p, s) =
            resolve_fixed_selection(None, Some(&morning(11, EARLY)), Some(&first_sweep(6, LATE)));
        assert_eq!(p.axis, Some(hn(11)));
        assert!(matches!(s, FixSource::Morning { .. }));
    }

    #[test]
    fn morning_computed_in_the_same_sweep_still_wins() {
        // 同じスイープで計算・評価したとき、computed_at と captured_at は同じ値になる
        let (p, _) =
            resolve_fixed_selection(None, Some(&morning(11, LATE)), Some(&first_sweep(6, LATE)));
        assert_eq!(p.axis, Some(hn(11)));
    }

    #[test]
    fn morning_saved_after_the_first_sweep_does_not_replace_it() {
        // T-40 で固定した後に朝の選定が現れても、固定は替わらない（軸フリップ禁止）
        let (p, s) =
            resolve_fixed_selection(None, Some(&morning(11, LATE)), Some(&first_sweep(6, EARLY)));
        assert_eq!(p.axis, Some(hn(6)));
        assert_eq!(p.partners, Some(vec![hn(6), hn(7), hn(8), hn(9), hn(10)]));
        assert!(matches!(s, FixSource::FirstSweep { .. }));
    }

    #[test]
    fn unparsable_times_fall_back_to_the_first_sweep() {
        let (p, _) = resolve_fixed_selection(
            None,
            Some(&morning(11, "garbage")),
            Some(&first_sweep(6, EARLY)),
        );
        assert_eq!(p.axis, Some(hn(6)));
    }

    #[test]
    fn first_sweep_is_the_fallback_without_morning() {
        let (p, s) = resolve_fixed_selection(None, None, Some(&first_sweep(6, EARLY)));
        assert_eq!(p.axis, Some(hn(6)));
        assert_eq!(
            p.konsen_band,
            Some(vec![hn(6), hn(7), hn(8), hn(9)]),
            "混戦の帯も固定"
        );
        assert!(matches!(
            s,
            FixSource::FirstSweep {
                recorded: false,
                ..
            }
        ));
    }

    #[test]
    fn recorded_axis_after_the_morning_selection_is_not_used() {
        // 朝の選定を保存した後に入った記録◎（3）は使わない（盤と predict-watch で固定をそろえる）
        let (p, s) = resolve_fixed_selection(Some(hn(3)), Some(&morning(11, EARLY)), None);
        assert_eq!(p.axis, Some(hn(11)));
        assert_eq!(
            p.partners.as_ref().map(Vec::len),
            Some(5),
            "相手は朝の選定のまま"
        );
        assert!(matches!(
            s,
            FixSource::Morning {
                recorded: false,
                ..
            }
        ));
        // 朝の選定を組んだときに記録◎ を強制していれば（軸が一致）、出所に記録◎ と出せる
        let (_, s) = resolve_fixed_selection(Some(hn(11)), Some(&morning(11, EARLY)), None);
        assert!(matches!(s, FixSource::Morning { recorded: true, .. }));
    }

    fn views(wins: &[(u32, f64)]) -> PredictionViews {
        let probs: Vec<HorseProbability> = wins
            .iter()
            .map(|(n, w)| HorseProbability {
                horse_num: hn(*n),
                horse_name: HorseName::try_from(format!("ウマ{n}")).unwrap(),
                win_prob: *w,
                place_prob: 0.0,
                show_prob: 0.0,
            })
            .collect();
        PredictionViews {
            blended: probs.clone(),
            pure: probs,
            explanations: Vec::new(),
            recent_runs_coverage: RecentRunsCoverage {
                field_size: wins.len(),
                horses_with_runs: wins.len(),
            },
        }
    }

    fn win_only(nums: &[u32]) -> RaceOdds {
        let mut o = RaceOdds::empty(RaceId::try_from("2026-4-tokyo-4-11R").unwrap());
        for n in nums {
            o.win
                .insert(hn(*n), OddsValue::try_from((BetType::Win, 5.0)).unwrap());
        }
        o
    }

    #[test]
    fn recorded_axis_overrides_only_the_first_sweep_axis() {
        // 朝の選定が無く初回スイープで固定しているレースでは、従来どおり記録◎ が軸だけ上書きする（#601）
        let (p, s) = resolve_fixed_selection(Some(hn(3)), None, Some(&first_sweep(6, EARLY)));
        assert_eq!(p.axis, Some(hn(3)));
        assert_eq!(p.partners, Some(vec![hn(6), hn(7), hn(8), hn(9), hn(10)]));
        assert!(matches!(s, FixSource::FirstSweep { recorded: true, .. }));
    }

    #[test]
    fn pick_selection_forces_the_recorded_axis_and_keeps_five_partners() {
        let v = views(&[
            (1, 0.30),
            (2, 0.20),
            (3, 0.15),
            (4, 0.12),
            (5, 0.10),
            (6, 0.08),
            (7, 0.05),
        ]);
        let odds = win_only(&[1, 2, 3, 4, 5, 6, 7]);
        let free = pick_selection(&v, &odds, None, MORNING_SELECTION_BUDGET);
        assert_eq!(free.axis, Some(hn(1)));

        let forced = pick_selection(&v, &odds, Some(hn(4)), MORNING_SELECTION_BUDGET);
        assert_eq!(forced.axis, Some(hn(4)));
        let mut partners = forced.partners.unwrap();
        partners.sort_by_key(|h| h.value());
        assert_eq!(
            partners,
            vec![hn(1), hn(2), hn(3), hn(5), hn(6)],
            "記録◎ を軸にしても相手は 5 頭で、モデルの首位も残る"
        );
    }

    #[test]
    fn jst_midnight_is_previous_day_15_utc() {
        let d = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap();
        assert_eq!(
            jst_midnight_utc(d).to_rfc3339(),
            "2026-08-14T15:00:00+00:00"
        );
    }
}
