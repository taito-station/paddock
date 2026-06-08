use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{BacktestReport, HorseEntry, HorseFactors, RaceEvaluation, evaluate};

use crate::error::Result;
use crate::interactor::Interactor;
use crate::interactor::race::predict::build_factors;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::Repository;

impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 指定期間 `[from, to]` の確定済みレースに対して確率推定を再現し、予測と実着順を突合した
    /// バックテストレポートを返す。各レース日 D の統計は `as_of = Some(D)`（`races.date < D`）で
    /// 取得するため、評価対象レース当日・以降の結果はリークしない（walk-forward）。
    pub async fn backtest(&self, from: NaiveDate, to: NaiveDate) -> Result<BacktestReport> {
        let races = self
            .repository
            .find_finished_races_between(from, to)
            .await?;

        let mut evaluations: Vec<RaceEvaluation> = Vec::with_capacity(races.len());
        for race in &races {
            if race.results.is_empty() {
                continue;
            }
            let as_of = Some(race.date);

            // コース統計は全馬共通なのでループ外で 1 回だけ取得する（predict と同じ）。
            let course = self
                .repository
                .course_stats(race.venue, race.distance, race.surface, as_of)
                .await?;

            let mut entry_factors: Vec<(HorseEntry, HorseFactors)> = Vec::new();
            for r in &race.results {
                let entry = HorseEntry {
                    gate_num: r.gate_num,
                    horse_num: r.horse_num,
                    horse_name: r.horse_name.clone(),
                    jockey: r.jockey.clone(),
                };
                let horse = self.repository.horse_stats(&r.horse_name, as_of).await?;
                let jockey = match &r.jockey {
                    Some(j) => Some(self.repository.jockey_stats(j, as_of).await?),
                    None => None,
                };
                let factors = build_factors(
                    &entry,
                    &course,
                    &horse,
                    jockey.as_ref(),
                    race.surface,
                    race.distance,
                );
                entry_factors.push((entry, factors));
            }

            let probs = paddock_domain::prediction::estimate_probabilities(&entry_factors);
            if probs.is_empty() {
                continue;
            }

            // 馬番 → (着順, 単勝オッズ) の突合表。
            let by_num: HashMap<u32, (Option<u32>, Option<f64>)> = race
                .results
                .iter()
                .map(|r| {
                    (
                        r.horse_num.value(),
                        (r.finishing_position.map(|p| p.value()), r.odds),
                    )
                })
                .collect();

            // 全馬の (win_prob, 1 着か) を蓄積（Brier / LogLoss 用）。
            let win_outcomes: Vec<(f64, bool)> = probs
                .iter()
                .map(|p| {
                    let won = by_num.get(&p.horse_num.value()).and_then(|(pos, _)| *pos) == Some(1);
                    (p.win_prob, won)
                })
                .collect();

            // トップ選好馬: win_prob 最大、同値は馬番昇順。
            let top = probs
                .iter()
                .reduce(|a, b| {
                    if b.win_prob > a.win_prob
                        || (b.win_prob == a.win_prob && b.horse_num.value() < a.horse_num.value())
                    {
                        b
                    } else {
                        a
                    }
                })
                .expect("probs is non-empty");
            let (top_pick_position, top_pick_odds) = by_num
                .get(&top.horse_num.value())
                .copied()
                .unwrap_or((None, None));

            evaluations.push(RaceEvaluation {
                win_outcomes,
                top_pick_position,
                top_pick_odds,
            });
        }

        Ok(evaluate(&evaluations))
    }
}
