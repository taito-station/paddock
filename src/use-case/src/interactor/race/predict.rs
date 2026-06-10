use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{
    HorseEntry, HorseFactors, HorseName, HorseProbability, RaceId, RateTriple, Surface,
    TrackCondition,
};

use crate::error::{Error, Result};
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{CourseStatsRow, GroupStat, HorseStatsRow, JockeyStatsRow, Repository};

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

        let race_ctx = RaceContext {
            surface: card.surface,
            distance: card.distance,
            track_condition,
        };
        let mut entry_factors: Vec<(HorseEntry, HorseFactors)> = Vec::new();
        for entry in &card.entries {
            let horse = self.repository.horse_stats(&entry.horse_name, None).await?;
            // jockey が None の馬は jockey 項を母数から除外して重み付き平均で評価され、欠落で
            // 不当に減点されない（ADR 0007）。
            let jockey = match &entry.jockey {
                Some(j) => Some(self.repository.jockey_stats(j, None).await?),
                None => None,
            };
            // 前走フォーム（#31）。前走は出馬表日より前の成績から取る（card.date が cutoff）。
            let recent_form = self.recent_form_for(&entry.horse_name, card.date).await?;
            let factors = build_factors(
                entry,
                &course,
                &horse,
                jockey.as_ref(),
                &race_ctx,
                recent_form,
            );
            entry_factors.push((entry.clone(), factors));
        }

        // estimate_probabilities が win→1.0 / place→2.0 / show→3.0 正規化 + 累積 max 単調化を行い、
        // win_prob ≤ place_prob ≤ show_prob を保証する（ADR 0007）。
        let probs = paddock_domain::prediction::estimate_probabilities(&entry_factors);

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
    /// 有効な signal が無い場合は `None`。predict と backtest で共有する（#31）。
    pub(crate) async fn recent_form_for(
        &self,
        name: &HorseName,
        before: NaiveDate,
    ) -> Result<Option<f64>> {
        let runs = self.repository.find_recent_runs(name, before, 1).await?;
        Ok(runs
            .first()
            .and_then(|(d, r)| paddock_domain::prediction::recent_form_score(r, *d, before)))
    }
}

/// `build_factors` に渡すレース側の条件（全馬共通）。
pub(crate) struct RaceContext {
    pub surface: Surface,
    pub distance: u32,
    /// 評価対象レースの馬場状態（#73）。未確定なら `None`（馬場項なし）。
    pub track_condition: Option<TrackCondition>,
}

/// 取得済みの stats 行と前走フォームから `HorseFactors` を組み立てる純粋変換。`as_of` には
/// 依存しないため、本番 predict（全期間統計）とバックテスト（as-of 統計）の両方から共有する。
/// `recent_form` は呼び出し側が前走から算出して渡す（#31）。
pub(crate) fn build_factors(
    entry: &HorseEntry,
    course: &CourseStatsRow,
    horse: &HorseStatsRow,
    jockey: Option<&JockeyStatsRow>,
    race: &RaceContext,
    recent_form: Option<f64>,
) -> HorseFactors {
    let gate_label = gate_group_label(entry.gate_num.value());
    let surf_label = surface_label(race.surface);
    let dist_label = distance_band_label(race.distance);

    HorseFactors {
        course_gate: stat_to_triple(&course.by_gate_group, gate_label),
        horse_surface: stat_to_triple(&horse.by_surface, surf_label),
        horse_distance: stat_to_triple(&horse.by_distance_band, dist_label),
        jockey_surface: jockey.map(|j| stat_to_triple(&j.by_surface, surf_label)),
        // 馬場状態が未確定のレース・該当馬場での出走実績が無い馬は None（項と重みを母数から
        // 除外、ADR 0007 の欠落項扱い）。0 埋め（stat_to_triple）にすると実績なしが減点に
        // なるため、ここは Option で区別する（#73）。
        horse_track_condition: race
            .track_condition
            .and_then(|tc| stat_to_triple_opt(&horse.by_track_condition, tc.as_str())),
        recent_form,
    }
}

/// 一致なし・出走 0 件は 0 レートに畳む。`starts == 0` は `GroupStat` の rate メソッドが
/// 0.0 を返すため、`stat_to_triple_opt` 導入前（label 一致のみで変換）と挙動同値。
fn stat_to_triple(groups: &[GroupStat], label: &str) -> RateTriple {
    stat_to_triple_opt(groups, label).unwrap_or_default()
}

/// label 一致の GroupStat を RateTriple へ変換する。一致なし・出走 0 件は `None` を返し、
/// 呼び出し側が「実績なし」を 0 レートと区別できるようにする（#73）。
/// 前提: groups 内で label は一意（rdb-gateway の `group_by` が固定キーごとに 1 行生成する）。
fn stat_to_triple_opt(groups: &[GroupStat], label: &str) -> Option<RateTriple> {
    groups
        .iter()
        .find(|g| g.label == label && g.starts > 0)
        .map(|g| RateTriple {
            win: g.win_rate(),
            place: g.place_rate(),
            show: g.show_rate(),
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
