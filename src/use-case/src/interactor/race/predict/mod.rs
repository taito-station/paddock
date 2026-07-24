//! レース予想（確率推定）の interactor。従来 `predict.rs` 単一ファイルに同居していた
//! オーケストレーション・特徴量関数・説明生成を、import パスを変えずに機械的に分割したもの（#454）。
//! - [`orchestrate`]: `Interactor` メソッド（`predict_race` / `predict_race_views*`）と factor 収集・推定。
//! - [`features`]: 前走フォーム・脚質・共有 factor など純粋な特徴量関数（backtest 経路と共有）。
//! - [`explain`]: 予想根拠 [`paddock_domain::HorseExplanation`] の組み立て。
//!
//! `pub(crate)` の特徴量関数（`build_factors` 等）と `PredictionViews` は backtest.rs / lib.rs が
//! `crate::interactor::race::predict::X` の従来パスで参照するため、ここで re-export して維持する。

pub mod explain;
pub mod features;
pub mod orchestrate;

pub use orchestrate::PredictionViews;

// backtest.rs が `crate::interactor::race::predict::{...}` で参照する特徴量関数・型（#196 共有）。
pub(crate) use features::{
    HorseSignals, RaceContext, TREND_WEIGHTS, build_factors, field_mean_weight,
    recent_form_from_runs, resolve_shared_factors, running_style_from_runs,
};

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use paddock_domain::{
        ExplainCategory, JockeyName, RecentRun, StandardTimes, Surface, Venue, Verdict,
        horse_result::{
            FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, ResultStatus,
        },
        race_card::HorseEntry,
    };

    use super::explain::build_explanation;
    use super::{
        RaceContext, recent_form_from_runs, resolve_shared_factors, running_style_from_runs,
    };
    use crate::repository::{CourseStatsRow, GroupStat, HorseStatsRow, JockeyStatsRow};

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// shows/starts から GroupStat を作る（win/place は show と整合する適当値で埋める）。
    fn group(label: &str, starts: u32, shows: u32) -> GroupStat {
        GroupStat {
            label: label.to_string(),
            starts,
            wins: shows / 3,
            places: shows / 2,
            shows,
        }
    }

    fn empty_horse_stats() -> HorseStatsRow {
        HorseStatsRow {
            horse_name: "テスト".to_string(),
            by_surface: vec![],
            by_distance_band: vec![],
            by_gate_group: vec![],
            by_track_condition: vec![],
            by_popularity_band: vec![],
            by_venue: vec![],
            by_jockey: vec![],
            overall: GroupStat::new("overall"),
        }
    }

    #[test]
    fn build_explanation_maps_factors_prev_and_weight() {
        let entry = HorseEntry {
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(5u32).unwrap(),
            horse_name: HorseName::try_from("テスト馬").unwrap(),
            jockey: None,
            trainer: None,
            weight_carried: Some(57.0),
        };
        let course = CourseStatsRow {
            venue: "東京".to_string(),
            distance: 1600,
            surface: "芝".to_string(),
            by_gate_group: vec![group("Inner (1-3)", 100, 23)],
        };
        let horse = HorseStatsRow {
            // 芝 複勝率 50%（10走）→ 縮約後も prior 超で Strong
            by_surface: vec![group("芝", 10, 5)],
            // 1500〜1800m 複勝率 0%（8走）→ Weak
            by_distance_band: vec![group("1500〜1800m", 8, 0)],
            ..empty_horse_stats()
        };
        let race = RaceContext {
            venue: Venue::Tokyo,
            surface: Surface::Turf,
            distance: 1600,
            track_condition: None, // 馬場 factor なし
            field_size: 16,
            mean_weight: Some(55.0),
        };
        let prev = RecentRun {
            date: ymd(2026, 1, 6),
            surface: Surface::Turf,
            distance: 1600,
            result: HorseResult {
                finishing_position: Some(FinishingPosition::try_from(3u32).unwrap()),
                status: ResultStatus::Finished,
                gate_num: GateNum::try_from(1u32).unwrap(),
                horse_num: HorseNum::try_from(5u32).unwrap(),
                horse_name: HorseName::try_from("テスト馬").unwrap(),
                horse_id: None,
                jockey: None,
                trainer: None,
                time_seconds: None,
                margin: None,
                odds: None,
                horse_weight: None,
                weight_change: None,
                weight_carried: None,
                popularity: Some(8),
            },
            corner_positions: None,
            field_size: None,
        };

        let shared = resolve_shared_factors(&entry, &course, &horse, None, None, &race);
        let ex = build_explanation(&shared, &entry, None, &race, Some(0.7), Some(&prev));

        // jockey/trainer/馬場 は None なので factor は 芝・距離・枠 の 3 本。
        assert_eq!(ex.factors.len(), 3);
        assert_eq!(ex.factors[0].category, ExplainCategory::Surface);
        assert_eq!(ex.factors[0].label, "芝");
        assert_eq!(ex.factors[0].verdict, Some(Verdict::Strong));
        assert_eq!(ex.factors[1].category, ExplainCategory::Distance);
        assert_eq!(ex.factors[1].label, "1500〜1800m");
        assert_eq!(ex.factors[1].verdict, Some(Verdict::Weak));
        assert_eq!(ex.factors[2].category, ExplainCategory::CourseGate);
        assert_eq!(ex.factors[2].label, "Inner (1-3)");
        // 枠は全馬横断率なので verdict なし。
        assert_eq!(ex.factors[2].verdict, None);

        assert_eq!(ex.recent_form, Some(0.7));
        let prev_summary = ex.prev_run.expect("前走サマリがあるはず");
        assert_eq!(prev_summary.finishing_position, Some(3));
        assert_eq!(prev_summary.popularity, Some(8));
        assert_eq!(ex.weight_carried, Some(57.0));
        assert_eq!(ex.field_mean_weight, Some(55.0));
    }

    #[test]
    fn build_explanation_adds_affinity_factors() {
        // #366(b): 相性 factor（騎手×場/距離・馬×騎手コンビ・馬×場）が実績ありのとき push され、
        // いずれも率のみ提示（verdict=None）であることを確認する。馬の芝ダ/距離/枠は空にして相性のみ拾う。
        let entry = HorseEntry {
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(5u32).unwrap(),
            horse_name: HorseName::try_from("テスト馬").unwrap(),
            jockey: Some(JockeyName::try_from("武豊".to_string()).unwrap()),
            trainer: None,
            weight_carried: None,
        };
        let course = CourseStatsRow {
            venue: "東京".to_string(),
            distance: 1600,
            surface: "芝".to_string(),
            by_gate_group: vec![],
        };
        let horse = HorseStatsRow {
            by_jockey: vec![group("武豊", 8, 4)], // 馬×騎手コンビ
            by_venue: vec![group("東京", 12, 4)], // 馬×場（当場）
            ..empty_horse_stats()
        };
        let jockey = JockeyStatsRow {
            jockey_name: "武豊".to_string(),
            overall: GroupStat::new("overall"),
            by_surface: vec![],
            by_gate_group: vec![],
            by_venue: vec![group("東京", 40, 11)], // 騎手×場
            by_distance_band: vec![group("1500〜1800m", 50, 14)], // 騎手×距離
        };
        let race = RaceContext {
            venue: Venue::Tokyo,
            surface: Surface::Turf,
            distance: 1600,
            track_condition: None,
            field_size: 16,
            mean_weight: None,
        };

        let shared = resolve_shared_factors(&entry, &course, &horse, Some(&jockey), None, &race);
        let ex = build_explanation(&shared, &entry, None, &race, None, None);

        let cats: Vec<_> = ex
            .factors
            .iter()
            .map(|f| (f.category, f.label.as_str(), f.verdict))
            .collect();
        assert!(
            cats.contains(&(ExplainCategory::JockeyVenue, "東京", None)),
            "{cats:?}"
        );
        assert!(
            cats.contains(&(ExplainCategory::JockeyDistance, "1500〜1800m", None)),
            "{cats:?}"
        );
        assert!(
            cats.contains(&(ExplainCategory::JockeyHorseCombo, "武豊", None)),
            "{cats:?}"
        );
        assert!(
            cats.contains(&(ExplainCategory::HorseVenue, "東京", None)),
            "{cats:?}"
        );
    }

    #[test]
    fn build_explanation_first_starter_has_no_factors_or_prev() {
        // 初戦馬: stats 全空・前走なし・斤量なし → factors 空・prev_run None。
        let entry = HorseEntry {
            gate_num: GateNum::try_from(4u32).unwrap(),
            horse_num: HorseNum::try_from(2u32).unwrap(),
            horse_name: HorseName::try_from("新馬").unwrap(),
            jockey: None,
            trainer: None,
            weight_carried: None,
        };
        let course = CourseStatsRow {
            venue: "東京".to_string(),
            distance: 1600,
            surface: "芝".to_string(),
            by_gate_group: vec![], // コース統計も空
        };
        let race = RaceContext {
            venue: Venue::Tokyo,
            surface: Surface::Turf,
            distance: 1600,
            track_condition: None,
            field_size: 8,
            mean_weight: None,
        };
        let horse = empty_horse_stats();
        let shared = resolve_shared_factors(&entry, &course, &horse, None, None, &race);
        let ex = build_explanation(&shared, &entry, None, &race, None, None);
        assert!(ex.factors.is_empty());
        assert!(ex.prev_run.is_none());
        assert_eq!(ex.weight_carried, None);
    }

    #[test]
    fn build_explanation_adds_conditional_gate_bias_with_lift() {
        use crate::repository::{ConditionalGateStatsRow, GateBiasCell};
        use paddock_domain::TrackCondition;
        // 良・16頭(多帯)・内枠。当該セル複勝40%、中/外は20%/15% → 全枠平均=25%、lift=+15pt。
        let mk = |gate: &str, shows: u32| GateBiasCell {
            track_label: "良".to_string(),
            field_label: "多(14-18)".to_string(),
            gate_label: gate.to_string(),
            stat: group(gate, 100, shows),
        };
        let cg = ConditionalGateStatsRow {
            cells: vec![
                mk("Inner (1-3)", 40),
                mk("Middle (4-6)", 20),
                mk("Outer (7-8)", 15),
            ],
        };
        let entry = HorseEntry {
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(3u32).unwrap(),
            horse_name: HorseName::try_from("枠テスト").unwrap(),
            jockey: None,
            trainer: None,
            weight_carried: None,
        };
        let course = CourseStatsRow {
            venue: "東京".to_string(),
            distance: 1600,
            surface: "芝".to_string(),
            by_gate_group: vec![],
        };
        let race = RaceContext {
            venue: Venue::Tokyo,
            surface: Surface::Turf,
            distance: 1600,
            track_condition: Some(TrackCondition::Firm),
            field_size: 16,
            mean_weight: None,
        };
        let horse = empty_horse_stats();
        let shared = resolve_shared_factors(&entry, &course, &horse, None, None, &race);
        let ex = build_explanation(&shared, &entry, Some(&cg), &race, None, None);
        let f = ex
            .factors
            .iter()
            .find(|f| f.category == ExplainCategory::ConditionalGateBias)
            .expect("条件依存枠バイアスが提示される");
        assert!((f.rate.show - 0.40).abs() < 1e-9);
        assert_eq!(f.verdict, None, "全馬横断率＝判定なし");
        assert!(
            f.label.contains("内枠") && f.label.contains("良") && f.label.contains("多(14-18)"),
            "label={}",
            f.label
        );
        // lift = 0.40 − 0.25。
        assert!(
            (ex.gate_bias_lift.unwrap() - 0.15).abs() < 1e-9,
            "lift={:?}",
            ex.gate_bias_lift
        );
    }

    #[test]
    fn build_explanation_no_gate_bias_when_track_unconfirmed() {
        use crate::repository::{ConditionalGateStatsRow, GateBiasCell};
        let cg = ConditionalGateStatsRow {
            cells: vec![GateBiasCell {
                track_label: "良".to_string(),
                field_label: "多(14-18)".to_string(),
                gate_label: "Inner (1-3)".to_string(),
                stat: group("Inner (1-3)", 100, 40),
            }],
        };
        let entry = HorseEntry {
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(3u32).unwrap(),
            horse_name: HorseName::try_from("枠テスト").unwrap(),
            jockey: None,
            trainer: None,
            weight_carried: None,
        };
        let course = CourseStatsRow {
            venue: "東京".to_string(),
            distance: 1600,
            surface: "芝".to_string(),
            by_gate_group: vec![],
        };
        // 馬場未確定(None) → 枠バイアスは提示しない（当日馬場が定まらないと条件セルを引けない）。
        let race = RaceContext {
            venue: Venue::Tokyo,
            surface: Surface::Turf,
            distance: 1600,
            track_condition: None,
            field_size: 16,
            mean_weight: None,
        };
        let horse = empty_horse_stats();
        let shared = resolve_shared_factors(&entry, &course, &horse, None, None, &race);
        let ex = build_explanation(&shared, &entry, Some(&cg), &race, None, None);
        assert!(
            !ex.factors
                .iter()
                .any(|f| f.category == ExplainCategory::ConditionalGateBias)
        );
        assert_eq!(ex.gate_bias_lift, None);
    }

    #[test]
    fn gate_field_band_label_covers_boundaries() {
        use crate::repository::gate_field_band_label;
        // 少(≤9) / 中(10-13) / 多(14-18) の境界。集計 SQL の GATE_FIELD_BANDS と同一定数源。
        assert_eq!(gate_field_band_label(9), "少(-9)");
        assert_eq!(gate_field_band_label(10), "中(10-13)");
        assert_eq!(gate_field_band_label(13), "中(10-13)");
        assert_eq!(gate_field_band_label(14), "多(14-18)");
        assert_eq!(gate_field_band_label(18), "多(14-18)");
    }

    #[test]
    fn gate_track_cond2_label_maps_firm_vs_rest() {
        use crate::repository::gate_track_cond2_label;
        use paddock_domain::TrackCondition;
        assert_eq!(
            gate_track_cond2_label(TrackCondition::Firm.to_string().as_str()),
            "良"
        );
        for tc in [
            TrackCondition::Good,
            TrackCondition::Yielding,
            TrackCondition::Soft,
        ] {
            assert_eq!(
                gate_track_cond2_label(tc.to_string().as_str()),
                "非良",
                "{tc:?} は非良"
            );
        }
    }

    #[test]
    fn condition_show_rate_none_when_no_starts() {
        use crate::repository::{ConditionalGateStatsRow, GateBiasCell};
        let row = ConditionalGateStatsRow {
            cells: vec![GateBiasCell {
                track_label: "良".to_string(),
                field_label: "少(-9)".to_string(),
                gate_label: "Inner (1-3)".to_string(),
                stat: group("Inner (1-3)", 0, 0),
            }],
        };
        assert_eq!(row.condition_show_rate("良", "少(-9)"), None);
    }

    fn run_valid(date: NaiveDate, weight_change: Option<i32>) -> RecentRun {
        RecentRun {
            date,
            surface: Surface::Turf,
            distance: 1600,
            result: HorseResult {
                finishing_position: Some(FinishingPosition::try_from(1u32).unwrap()),
                status: ResultStatus::Finished,
                gate_num: GateNum::try_from(1u32).unwrap(),
                horse_num: HorseNum::try_from(1u32).unwrap(),
                horse_name: HorseName::try_from("テスト").unwrap(),
                horse_id: None,
                jockey: None,
                trainer: None,
                time_seconds: None,
                margin: None,
                odds: None,
                horse_weight: None,
                weight_change,
                weight_carried: None,
                popularity: None,
            },
            corner_positions: None,
            field_size: None,
        }
    }

    /// score=None になる走を生成するヘルパー。
    /// `date=before`（cutoff 当日）で days=0 → scoring.rs が間隔シグナルを落とす。
    /// `status=DidNotFinish` + `weight_change=None` で着順・体重変化シグナルも落ちる。
    /// いずれか単独でも score=None になるが、二重に確保することでテストの堅牢性を高めている。
    fn run_no_score(before: NaiveDate) -> RecentRun {
        RecentRun {
            date: before,
            surface: Surface::Turf,
            distance: 1600,
            result: HorseResult {
                finishing_position: None,
                status: ResultStatus::DidNotFinish,
                gate_num: GateNum::try_from(1u32).unwrap(),
                horse_num: HorseNum::try_from(1u32).unwrap(),
                horse_name: HorseName::try_from("テスト").unwrap(),
                horse_id: None,
                jockey: None,
                trainer: None,
                time_seconds: None,
                margin: None,
                odds: None,
                horse_weight: None,
                weight_change: None,
                weight_carried: None,
                popularity: None,
            },
            corner_positions: None,
            field_size: None,
        }
    }

    #[test]
    fn trend_n2_both_valid_weighted_average() {
        let before = ymd(2026, 1, 20);
        // 14 日前（interval_form=1.0: scoring.rs の 14〜60 日帯）・weight_change=0（signal=1.0） → score = 1.0
        let run1 = run_valid(ymd(2026, 1, 6), Some(0));
        // 28 日前（interval_form=1.0: 同 14〜60 日帯）・weight_change=20=WEIGHT_CHANGE_CAP（上限境界値: signal=0.0） → score = 0.5
        let run2 = run_valid(ymd(2025, 12, 23), Some(20));
        let st = StandardTimes::default();
        let result = recent_form_from_runs(&[run1, run2], before, &st, 2).unwrap();
        // wsum = 1.0*1.0 + 0.5*0.5 = 1.25, wden = 1.5 → 1.25/1.5
        // 期待値は scoring.rs の WEIGHT_CHANGE_CAP=20.0・interval_form 14〜60 日=1.0 に依存（scoring 変更時は要確認）。
        let expected = 1.25_f64 / 1.5;
        assert!(
            (result - expected).abs() < 1e-9,
            "got {result}, expected {expected}"
        );
    }

    #[test]
    fn trend_n2_second_run_no_score_uses_first_only() {
        let before = ymd(2026, 1, 20);
        let run1 = run_valid(ymd(2026, 1, 6), Some(0)); // score=1.0
        let run2 = run_no_score(before); // score=None
        let st = StandardTimes::default();
        let result = recent_form_from_runs(&[run1, run2], before, &st, 2).unwrap();
        // wsum = 1.0, wden = 1.0 → 1.0
        assert!((result - 1.0).abs() < 1e-9, "got {result}");
    }

    #[test]
    fn trend_n2_all_no_score_returns_none() {
        let before = ymd(2026, 1, 20);
        let runs = vec![run_no_score(before), run_no_score(before)];
        let st = StandardTimes::default();
        assert!(recent_form_from_runs(&runs, before, &st, 2).is_none());
    }

    #[test]
    fn trend_n1_uses_only_first_run() {
        let before = ymd(2026, 1, 20);
        let run1 = run_valid(ymd(2026, 1, 6), Some(0)); // score=1.0
        let run2 = run_valid(ymd(2025, 12, 23), Some(20)); // score=0.5 (would lower if included)
        let st = StandardTimes::default();
        let result = recent_form_from_runs(&[run1, run2], before, &st, 1).unwrap();
        assert!((result - 1.0).abs() < 1e-9, "got {result}");
    }

    #[test]
    fn trend_n3_all_valid_uses_all_weights() {
        let before = ymd(2026, 1, 20);
        // score=1.0, 1.0, 0.5 の 3 走 → wsum=1.0*1+0.5*1+0.25*0.5=1.625, wden=1.75
        let run1 = run_valid(ymd(2026, 1, 6), Some(0)); // score=1.0
        let run2 = run_valid(ymd(2025, 12, 23), Some(0)); // score=1.0
        let run3 = run_valid(ymd(2025, 12, 9), Some(20)); // score=0.5
        let st = StandardTimes::default();
        let result = recent_form_from_runs(&[run1, run2, run3], before, &st, 3).unwrap();
        // wsum=1.0*1+0.5*1+0.25*0.5=1.625, wden=1.75
        // 期待値は scoring.rs の WEIGHT_CHANGE_CAP=20.0・interval_form 14〜60 日=1.0 に依存（scoring 変更時は要確認）。
        let expected = 1.625_f64 / 1.75;
        assert!(
            (result - expected).abs() < 1e-9,
            "got {result}, expected {expected}"
        );
    }

    /// 脚質テスト用に corner_positions/field_size だけ差し替えた RecentRun を作る。
    fn run_with_style(corner: Option<&str>, field_size: Option<u32>) -> RecentRun {
        RecentRun {
            corner_positions: corner.map(str::to_string),
            field_size,
            ..run_no_score(ymd(2026, 1, 1))
        }
    }

    #[test]
    fn running_style_from_runs_averages_valid_and_skips_invalid() {
        // 16 頭立て 1 番手（先行度 1.0）と 8 頭立て 8 番手（先行度 0.0）→ 平均 0.5。
        // corner/頭数を欠く走・正規化不能な走は母数から除外する。
        let runs = [
            run_with_style(Some("1-1"), Some(16)), // 1.0
            run_with_style(Some("8-7"), Some(8)),  // 0.0
            run_with_style(None, Some(16)),        // corner 欠落 → 除外
            run_with_style(Some("3-3"), None),     // 頭数欠落 → 除外
            run_with_style(Some(""), Some(16)),    // 解釈不能 → 除外
        ];
        let got = running_style_from_runs(&runs).expect("有効走 2 件で Some");
        assert!((got - 0.5).abs() < 1e-9, "got {got}");
    }

    #[test]
    fn running_style_from_runs_none_when_no_valid_run() {
        // 空・全欠落は None（母数除外・既存 scalar と統一）。
        assert!(running_style_from_runs(&[]).is_none());
        let runs = [
            run_with_style(None, Some(16)),
            run_with_style(Some("3-3"), None),
        ];
        assert!(running_style_from_runs(&runs).is_none());
    }
}
