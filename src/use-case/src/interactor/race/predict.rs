use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{
    EstimationConfig, FactorStat, HorseEntry, HorseFactors, HorseName, HorseProbability,
    JockeyName, RaceId, RateTriple, RecentRun, StandardTimes, Surface, TrackCondition, TrainerName,
};

use crate::error::{Error, Result};
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{
    CourseStatsRow, GroupStat, HorseRecencyStats, HorseStatsRow, JockeyStatsRow, OddsRepository,
    RaceCardRepository, RecencySeries, StatsRepository, TrainerStatsRow,
};

impl<R: StatsRepository + RaceCardRepository + OddsRepository, P: PdfParser, F: PdfFetcher>
    Interactor<R, P, F>
{
    /// 出馬表から各馬の win/place/show 確率を推定する。`blend_alpha = Some(α)` のとき、
    /// 当日の市場オッズ（単勝, `find_race_odds(.., None)` の最新スナップショット）の implied 確率と
    /// α（モデル重み）でブレンドする（#72）。`None` はモデルのみ（市場オッズを取得しない）。
    /// `track_condition = Some(..)` のとき、各馬の馬場状態別成績を factor に加える（#73）。
    /// 出馬表 PDF に馬場状態は無いため、呼び出し側が当日の値を渡す（未確定なら `None`）。
    pub async fn predict_race(
        &self,
        race_id: &RaceId,
        blend_alpha: Option<f64>,
        track_condition: Option<TrackCondition>,
    ) -> Result<Vec<HorseProbability>> {
        let card = self
            .repository
            .find_race_card(race_id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("race card: {}", race_id.value())))?;

        // コース統計は全馬共通なのでループ外で 1 回だけ取得する
        let course = self
            .repository
            .course_stats(card.venue, card.distance, card.surface, None)
            .await?;

        // 斤量のレース内相対シグナル用の field 平均斤量（#135）。斤量を持つ出走馬のみで平均する。
        // netkeiba 出馬表は斤量あり、PDF 出馬表は全馬 None なので平均も None（斤量項なし）。
        let mean_weight = field_mean_weight(card.entries.iter().filter_map(|e| e.weight_carried));
        let race_ctx = RaceContext {
            surface: card.surface,
            distance: card.distance,
            track_condition,
            mean_weight,
        };
        // 本番 predict の確率推定設定（#75: ベイズ縮約 m=10 を採用。recency は backtest 評価で
        // 改善が出ず無効のまま＝production() は recency: None。下の horse_recency も取得しない）。
        let config = paddock_domain::EstimationConfig::production();
        // 前走タイムの相対速度シグナル用の標準タイム表（#76）。全馬共通なのでループ外で 1 回だけ
        // 取得する。cutoff=card.date で出馬表日以降をリークさせない。
        let standard_times = self.repository.standard_times(card.date).await?;

        // 全馬・騎手・調教師の名前を収集して 4 クエリで一括取得する（per-horse N+1 解消 #205）。
        // 重複排除（同一騎手が複数馬に騎乗する場合等）は各 _batch 実装の内部で行うため、
        // 呼び出し側は重複ありで渡してよい。
        let horse_names: Vec<HorseName> =
            card.entries.iter().map(|e| e.horse_name.clone()).collect();
        let jockey_names: Vec<JockeyName> = card
            .entries
            .iter()
            .filter_map(|e| e.jockey.clone())
            .collect();
        let trainer_names: Vec<TrainerName> = card
            .entries
            .iter()
            .filter_map(|e| e.trainer.clone())
            .collect();
        // as_of: None = 全期間統計（predict は出馬表日時点での履歴制限なし。
        // リーク防止の as_of は backtest 経路のみ必要）。
        // try_join! の実際の並列度は接続プールのコネクション数に依存する。
        let (horse_map, jockey_map, trainer_map, runs_map) = tokio::try_join!(
            self.repository.horse_stats_batch(&horse_names, None),
            self.repository.jockey_stats_batch(&jockey_names, None),
            self.repository.trainer_stats_batch(&trainer_names, None),
            // limit: TREND_WEIGHTS の要素数まで取得し、trend_n で何走使うかを scoring 側で制御する（#220）。
            self.repository
                .recent_runs_batch(&horse_names, card.date, TREND_WEIGHTS.len() as u32),
        )?;

        let mut entry_factors: Vec<(HorseEntry, HorseFactors)> = Vec::new();
        for entry in &card.entries {
            // ok_or_else: batch 契約上 None になることはないが、rdb-gateway の override
            // バグを panic ではなく error として伝播させるため backtest の expect とは意図的に非対称。
            let horse = horse_map.get(&entry.horse_name).ok_or_else(|| {
                Error::NotFound(format!("horse stats: {}", entry.horse_name.value()))
            })?;
            // production() は recency: None なので horse_recency は取得しない（#75）。
            // jockey/trainer は DB 未登録（新人騎手・調教師交代等）が正当なケースのため horse と
            // 異なり ok_or_else でエラーにせず None とし母数から除外する（ADR 0007）。
            let jockey = entry.jockey.as_ref().and_then(|j| jockey_map.get(j));
            let trainer = entry.trainer.as_ref().and_then(|t| trainer_map.get(t));
            // 初戦馬（前走なし）は有効なケースのため unwrap_or(&[]) で空スライスを返す
            // （horse_map の ok_or_else との非対称は意図的）。
            let recent_runs = runs_map
                .get(&entry.horse_name)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let recent_form =
                recent_form_from_runs(recent_runs, card.date, &standard_times, config.trend_n);
            let factors = build_factors(
                entry,
                &course,
                horse,
                jockey,
                trainer,
                &race_ctx,
                recent_form,
                None, // recency: production() では無効
                card.date,
                &config,
            );
            entry_factors.push((entry.clone(), factors));
        }

        // estimate_probabilities が win→1.0 / place→2.0 / show→3.0 正規化 + 累積 max 単調化を行い、
        // win_prob ≤ place_prob ≤ show_prob を保証する（ADR 0007）。本番経路は #75 で採用した
        // ベイズ縮約（m=10）を有効にし、少データ馬の過信（win_prob=0 を含む）を緩和する。
        let probs =
            paddock_domain::prediction::estimate_probabilities_with_config(&entry_factors, &config);

        // 市場オッズ（単勝）ブレンド（#72）。α<1.0 のときのみ最新オッズスナップショットを取得する
        // （α>=1.0・非有限はブレンド無効なので DB クエリを省く）。
        let probs = match blend_alpha.filter(|a| a.is_finite() && *a < 1.0) {
            Some(alpha) => {
                let market = self.repository.find_race_odds(race_id, None).await?;
                match market {
                    Some(odds) => {
                        let market_win: HashMap<_, _> =
                            odds.win.iter().map(|(num, o)| (*num, o.value())).collect();
                        paddock_domain::prediction::blend_with_market_win(
                            &probs,
                            &market_win,
                            alpha,
                        )
                    }
                    None => probs,
                }
            }
            None => probs,
        };

        Ok(probs)
    }
}

/// 直近 N 走トレンドの重み（#220）。runs は date 降順なので index 0 が最新走。
/// `[1.0, 0.5, 0.25]` = Issue #220 指定の指数的減衰ウェイト。
/// `pub(crate)` にして backtest.rs からも参照できるようにする（バッチ取得上限と一致させるため）。
/// この配列を変更する場合は ADR-0036・CLI の `--trend-n` help・仕様書（probability-estimation.md）も更新すること。
pub(crate) const TREND_WEIGHTS: [f64; 3] = [1.0, 0.5, 0.25];

/// 取得済みの近走 `runs`（date 降順、最大 `limit` 件）から前走フォーム [0,1] を算出する純粋関数。
/// `recent_form_for` の DB 取得を剥がした本体で、backtest のバッチ取得（#196）からも共有する。
///
/// `trend_n = 1` のとき直近 1 走スコアのみ返し、現行挙動と完全一致する。
/// `trend_n >= 2` のとき有効スコアが得られた走を TREND_WEIGHTS で加重平均する。
/// スコアが取れなかった走（中止・情報欠落等）は分母から除外（欠落フォールバック維持）。
///
/// `before` は予測対象レースの日付（cutoff）。`recent_form_score` の間隔シグナルは
/// cutoff と各走の日付の差で算出するため N 走すべてに同じ `before` を渡す
/// （走間の間隔ではなく cutoff 基準）。リーク防止（before 以降の走を除外）も兼ねる。
/// `runs` は呼び出し元が `date < before` でフィルタ済みであることを前提とする
/// （この関数内では再チェックしない）。
/// `trend_n` は 1 以上でなければならない（CLI バリデーション済み）。
pub(crate) fn recent_form_from_runs(
    runs: &[RecentRun],
    before: NaiveDate,
    standard_times: &StandardTimes,
    trend_n: u32,
) -> Option<f64> {
    debug_assert!(trend_n >= 1, "trend_n must be >= 1, got {trend_n}");
    // 三重 min: trend_n 上界 → 近走実在数 → TREND_WEIGHTS 配列長（CLI バリデーション済みだが防衛的に維持）。
    let n = (trend_n as usize).min(runs.len()).min(TREND_WEIGHTS.len());
    let mut wsum = 0.0_f64;
    let mut wden = 0.0_f64;
    for (i, run) in runs[..n].iter().enumerate() {
        let std = standard_times.get(run.surface, run.distance);
        if let Some(s) =
            paddock_domain::prediction::recent_form_score(&run.result, run.date, before, std)
        {
            wsum += TREND_WEIGHTS[i] * s;
            wden += TREND_WEIGHTS[i];
        }
    }
    (wden > 0.0).then(|| wsum / wden)
}

/// `build_factors` に渡すレース側の条件（全馬共通）。
pub(crate) struct RaceContext {
    pub surface: Surface,
    pub distance: u32,
    /// 評価対象レースの馬場状態（#73）。未確定なら `None`（馬場項なし）。
    pub track_condition: Option<TrackCondition>,
    /// レース内の field 平均斤量[kg]（#135）。斤量を持つ出走馬が居ないレース（PDF 出馬表等）は
    /// `None`（斤量項なし）。`build_factors` が各馬の `weight_carried` との差から相対シグナルを作る。
    pub mean_weight: Option<f64>,
}

/// 取得済みの stats 行と前走フォームから `HorseFactors` を組み立てる純粋変換。本番 predict
/// （全期間統計）とバックテスト（as-of 統計）の両方から共有する。`recent_form` は呼び出し側が
/// 前走から算出して渡す（#31）。
///
/// `config.recency = Some(rc)` かつ `recency` が渡されたとき、馬自身の 3 factor（芝ダ・距離帯・
/// 馬場状態）は集計レートの代わりに日付付き系列を時間減衰した recency 重み付きレートで評価する
/// （#75 Phase B）。`as_of_date` は減衰の基準日（predict は出馬表日、backtest はレース日）。
/// course/jockey/trainer は従来の集計レートのまま。
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_factors(
    entry: &HorseEntry,
    course: &CourseStatsRow,
    horse: &HorseStatsRow,
    jockey: Option<&JockeyStatsRow>,
    trainer: Option<&TrainerStatsRow>,
    race: &RaceContext,
    recent_form: Option<f64>,
    recency: Option<&HorseRecencyStats>,
    as_of_date: NaiveDate,
    config: &EstimationConfig,
) -> HorseFactors {
    let gate_label = gate_group_label(entry.gate_num.value());
    let surf_label = surface_label(race.surface);
    let dist_label = distance_band_label(race.distance);

    // recency 有効時は horse 系 factor を日付系列の時間減衰で評価する。無効時・系列なしは集計レート。
    let recency_cfg = config.recency.zip(recency);
    let horse_surface = match recency_cfg {
        Some((rc, r)) => recency_factor(&r.by_surface, surf_label, as_of_date, rc.half_life_days),
        None => stat_to_triple_opt(&horse.by_surface, surf_label),
    };
    let horse_distance = match recency_cfg {
        Some((rc, r)) => recency_factor(
            &r.by_distance_band,
            dist_label,
            as_of_date,
            rc.half_life_days,
        ),
        None => stat_to_triple_opt(&horse.by_distance_band, dist_label),
    };
    let horse_track_condition = race.track_condition.and_then(|tc| match recency_cfg {
        Some((rc, r)) => recency_factor(
            &r.by_track_condition,
            tc.as_str(),
            as_of_date,
            rc.half_life_days,
        ),
        None => stat_to_triple_opt(&horse.by_track_condition, tc.as_str()),
    });

    // 全 factor で「実績なし」を None（母数除外）に統一する（#81/ADR 0014）。一致なし・出走 0 件は
    // stat_to_triple_opt が None を返し、0 レート（＝全敗）と区別される。jockey/trainer は
    // 騎手・調教師欠落（and_then の外側 None）と「実績なし」（内側 None）を二段で畳む。
    HorseFactors {
        course_gate: stat_to_triple_opt(&course.by_gate_group, gate_label),
        horse_surface,
        horse_distance,
        jockey_surface: jockey.and_then(|j| stat_to_triple_opt(&j.by_surface, surf_label)),
        trainer_surface: trainer.and_then(|t| stat_to_triple_opt(&t.by_surface, surf_label)),
        // 馬場状態が未確定のレース・該当馬場での出走実績が無い馬は None（#73）。
        horse_track_condition,
        recent_form,
        // 斤量のレース内相対シグナル（#135）。当該馬の斤量と field 平均斤量の両方があるときのみ項を立てる。
        // PDF 出馬表（斤量なし）・field 平均が出せないレースは None（母数除外）。
        weight_carried: entry
            .weight_carried
            .zip(race.mean_weight)
            .map(|(w, mean)| paddock_domain::prediction::weight_factor(w, mean)),
    }
}

/// 斤量のレース内相対シグナル用に、出走馬の斤量[kg]の単純平均を返す（#135）。有限値が 1 つも無ければ
/// `None`（斤量項なし）。predict（出馬表 entries）と backtest（出走馬 results）で共有する。
/// 非有限値（NaN/inf）は母数から除外し、1 件の異常値が平均を NaN 化して全馬の斤量項を汚染しないようにする。
pub(crate) fn field_mean_weight(weights: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = weights
        .filter(|w| w.is_finite())
        .fold((0.0, 0u32), |(s, c), w| (s + w, c + 1));
    (n > 0).then(|| sum / n as f64)
}

/// recency 系列からラベル一致の日付系列を取り、時間減衰した重み付きレート（[`FactorStat`]）を返す。
/// ラベル不一致・有効な過去走なしは `None`（集計経路の「実績なし＝母数除外」と同じ扱い）。
fn recency_factor(
    series: &[RecencySeries],
    label: &str,
    as_of: NaiveDate,
    half_life_days: f64,
) -> Option<FactorStat> {
    series
        .iter()
        .find(|s| s.label == label)
        .and_then(|s| paddock_domain::apply_recency_weight(&s.runs, as_of, half_life_days))
}

/// label 一致の GroupStat を `FactorStat`（レート + 出走数）へ変換する。一致なし・出走 0 件は
/// `None` を返し、呼び出し側が「実績なし」を 0 レートと区別できるようにする（#73 で導入、
/// #81 で全 factor 共通化）。`starts` はベイズ縮約の信頼度重みに使う（#75）。
/// 前提: groups 内で label は一意（rdb-gateway の `group_by` が固定キーごとに 1 行生成する）。
fn stat_to_triple_opt(groups: &[GroupStat], label: &str) -> Option<FactorStat> {
    groups
        .iter()
        .find(|g| g.label == label && g.starts > 0)
        .map(|g| FactorStat {
            rate: RateTriple {
                win: g.win_rate(),
                place: g.place_rate(),
                show: g.show_rate(),
            },
            starts: g.starts,
        })
}

fn surface_label(surface: Surface) -> &'static str {
    match surface {
        Surface::Turf => "芝",
        Surface::Dirt => "ダート",
    }
}

// GateNum は 1..=8 でバリデーション済みなので _ は常に 7-8 にのみ該当する
fn gate_group_label(gate_num: u32) -> &'static str {
    match gate_num {
        1..=3 => "Inner (1-3)",
        4..=6 => "Middle (4-6)",
        _ => "Outer (7-8)",
    }
}

// ラベルは group_by_distance_band の SQL ラベル文字列と完全一致させる。
// `<= 1800` / `<= 2200` と上限を基準にすることで、SQL の BETWEEN 境界と
// 実装の意図を揃える。JRA 実レース距離は 1400m・1600m・1800m・2000m・2200m・
// 2400m 等の離散値のみで、1401〜1499m のようなレースは存在しない。
fn distance_band_label(distance: u32) -> &'static str {
    if distance <= 1400 {
        "〜1400m"
    } else if distance <= 1800 {
        "1500〜1800m"
    } else if distance <= 2200 {
        "1900〜2200m"
    } else {
        "2300m〜"
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use paddock_domain::{
        RecentRun, StandardTimes, Surface,
        horse_result::{
            FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, ResultStatus,
        },
    };

    use super::recent_form_from_runs;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
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
}
