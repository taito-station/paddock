use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{
    HorseEntry, HorseFactors, HorseName, HorseProbability, RaceId, RateTriple, Surface,
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
    pub async fn predict_race(
        &self,
        race_id: &RaceId,
        blend_alpha: Option<f64>,
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
                card.surface,
                card.distance,
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

/// 取得済みの stats 行と前走フォームから `HorseFactors` を組み立てる純粋変換。`as_of` には
/// 依存しないため、本番 predict（全期間統計）とバックテスト（as-of 統計）の両方から共有する。
/// `recent_form` は呼び出し側が前走から算出して渡す（#31）。
pub(crate) fn build_factors(
    entry: &HorseEntry,
    course: &CourseStatsRow,
    horse: &HorseStatsRow,
    jockey: Option<&JockeyStatsRow>,
    surface: Surface,
    distance: u32,
    recent_form: Option<f64>,
) -> HorseFactors {
    let gate_label = gate_group_label(entry.gate_num.value());
    let surf_label = surface_label(surface);
    let dist_label = distance_band_label(distance);

    HorseFactors {
        course_gate: stat_to_triple(&course.by_gate_group, gate_label),
        horse_surface: stat_to_triple(&horse.by_surface, surf_label),
        horse_distance: stat_to_triple(&horse.by_distance_band, dist_label),
        jockey_surface: jockey.map(|j| stat_to_triple(&j.by_surface, surf_label)),
        recent_form,
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
