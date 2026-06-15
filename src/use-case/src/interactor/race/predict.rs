use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{
    EstimationConfig, FactorStat, HorseEntry, HorseFactors, HorseName, HorseProbability, RaceId,
    RateTriple, StandardTimes, Surface, TrackCondition,
};

use crate::error::{Error, Result};
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{
    CourseStatsRow, GroupStat, HorseRecencyStats, HorseStatsRow, JockeyStatsRow, RecencySeries,
    Repository, TrainerStatsRow,
};

impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
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
        let mut entry_factors: Vec<(HorseEntry, HorseFactors)> = Vec::new();
        for entry in &card.entries {
            let horse = self.repository.horse_stats(&entry.horse_name, None).await?;
            // recency 有効時のみ日付付き系列を取得する（#75 Phase B）。基準日は出馬表日。
            let recency = match config.recency {
                Some(_) => Some(
                    self.repository
                        .horse_recency(&entry.horse_name, None)
                        .await?,
                ),
                None => None,
            };
            // jockey が None の馬は jockey 項を母数から除外して重み付き平均で評価され、欠落で
            // 不当に減点されない（ADR 0007）。
            let jockey = match &entry.jockey {
                Some(j) => Some(self.repository.jockey_stats(j, None).await?),
                None => None,
            };
            // 調教師統計（#74）。netkeiba 出馬表由来の entry.trainer から引く。PDF 経路で
            // 取り込んだレースは entry.trainer=None のため項なし（ADR 0007 で減点しない）。
            let trainer = match &entry.trainer {
                Some(t) => Some(self.repository.trainer_stats(t, None).await?),
                None => None,
            };
            // 前走フォーム（#31）。前走は出馬表日より前の成績から取る（card.date が cutoff）。
            let recent_form = self
                .recent_form_for(&entry.horse_name, card.date, &standard_times)
                .await?;
            let factors = build_factors(
                entry,
                &course,
                &horse,
                jockey.as_ref(),
                trainer.as_ref(),
                &race_ctx,
                recent_form,
                recency.as_ref(),
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

    /// 指定馬の前走（`before` より前の直近 1 走）から前走フォーム [0,1] を算出する。前走が無い／
    /// 有効な signal が無い場合は `None`。predict と backtest で共有する（#31/#76）。`standard_times`
    /// は前走の (surface,distance) に対する標準タイムを引くための表（レース単位で 1 回取得して共有）。
    pub(crate) async fn recent_form_for(
        &self,
        name: &HorseName,
        before: NaiveDate,
        standard_times: &StandardTimes,
    ) -> Result<Option<f64>> {
        let runs = self.repository.find_recent_runs(name, before, 1).await?;
        Ok(runs.first().and_then(|run| {
            let std = standard_times.get(run.surface, run.distance);
            paddock_domain::prediction::recent_form_score(&run.result, run.date, before, std)
        }))
    }
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

/// 斤量のレース内相対シグナル用に、出走馬の斤量[kg]の単純平均を返す（#135）。値が 1 つも無ければ
/// `None`（斤量項なし）。predict（出馬表 entries）と backtest（出走馬 results）で共有する。
pub(crate) fn field_mean_weight(weights: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = weights.fold((0.0, 0u32), |(s, c), w| (s + w, c + 1));
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
