use paddock_domain::{
    HorseEntry, HorseFactors, HorseProbability, RaceId, RateTriple, Surface,
};

use crate::error::{Error, Result};
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{CourseStatsRow, GroupStat, HorseStatsRow, JockeyStatsRow, Repository};

impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    pub async fn predict_race(&self, race_id: &RaceId) -> Result<Vec<HorseProbability>> {
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

        let mut entry_factors: Vec<(HorseEntry, HorseFactors)> = Vec::new();
        for entry in &card.entries {
            let horse = self.repository.horse_stats(&entry.horse_name, None).await?;
            // jockey が None の馬は jockey_surface_rate = 0.0 として計算され、
        // jockey 情報がある馬と比べてスコアが相対的に低くなる（既知制約）
        let jockey = match &entry.jockey {
                Some(j) => Some(self.repository.jockey_stats(j, None).await?),
                None => None,
            };
            let factors = build_factors(
                entry,
                &course,
                &horse,
                jockey.as_ref(),
                card.surface,
                card.distance,
            );
            entry_factors.push((entry.clone(), factors));
        }

        // win / place / show はそれぞれ独立に正規化するため、
        // win_prob ≤ place_prob ≤ show_prob の単調性は保証されない（設計上の既知制約）
        Ok(paddock_domain::prediction::estimate_probabilities(&entry_factors))
    }
}

/// 取得済みの stats 行から `HorseFactors` を組み立てる純粋変換。`as_of` には依存しないため、
/// 本番 predict（全期間統計）とバックテスト（as-of 統計）の両方から共有する。
pub(crate) fn build_factors(
    entry: &HorseEntry,
    course: &CourseStatsRow,
    horse: &HorseStatsRow,
    jockey: Option<&JockeyStatsRow>,
    surface: Surface,
    distance: u32,
) -> HorseFactors {
    let gate_label = gate_group_label(entry.gate_num.value());
    let surf_label = surface_label(surface);
    let dist_label = distance_band_label(distance);

    HorseFactors {
        course_gate: stat_to_triple(&course.by_gate_group, gate_label),
        horse_surface: stat_to_triple(&horse.by_surface, surf_label),
        horse_distance: stat_to_triple(&horse.by_distance_band, dist_label),
        jockey_surface: jockey.map(|j| stat_to_triple(&j.by_surface, surf_label)),
    }
}

fn stat_to_triple(groups: &[GroupStat], label: &str) -> RateTriple {
    groups
        .iter()
        .find(|g| g.label == label)
        .map(|g| RateTriple {
            win: g.win_rate(),
            place: g.place_rate(),
            show: g.show_rate(),
        })
        .unwrap_or_default()
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
