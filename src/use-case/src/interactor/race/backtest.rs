use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{
    BacktestReport, HorseEntry, HorseFactors, HorseResult, RaceEvaluation, ResultStatus, evaluate,
};

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
    ///
    /// 性能: 統計取得は馬ごとに `horse_stats`/`jockey_stats` を逐次呼ぶ N+1 になる。`as_of` が
    /// レース日ごとに変わる walk-forward の性質上、馬名でまたいでバッチ化できないため許容する
    /// （オフライン評価用途で実行頻度が低い）。レース取得自体の N+1 は
    /// [`Repository::find_finished_races_between`] が 2 クエリで回避済み。
    pub async fn backtest(&self, from: NaiveDate, to: NaiveDate) -> Result<BacktestReport> {
        let races = self
            .repository
            .find_finished_races_between(from, to)
            .await?;

        let mut evaluations: Vec<RaceEvaluation> = Vec::with_capacity(races.len());
        for race in &races {
            // 実際に発走した馬のみを評価対象（出走頭数）にする。出走取消・競走除外は本番 predict の
            // 出馬表にも載らないため、確率推定の母集合に含めると正規化分母が水増しされ確率が歪む。
            // 競走中止(DidNotFinish)は発走済みなので母集合に含め、非的中(着順なし)として扱う。
            let starters: Vec<&HorseResult> = race
                .results
                .iter()
                .filter(|r| !matches!(r.status, ResultStatus::Scratched | ResultStatus::Cancelled))
                .collect();
            // 発走馬が居なければ評価できないのでスキップ。
            if starters.is_empty() {
                continue;
            }
            let as_of = Some(race.date);

            // コース統計は全馬共通なのでループ外で 1 回だけ取得する（predict と同じ）。
            let course = self
                .repository
                .course_stats(race.venue, race.distance, race.surface, as_of)
                .await?;

            let mut entry_factors: Vec<(HorseEntry, HorseFactors)> = Vec::new();
            for r in &starters {
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
                // 前走フォーム（#31）。cutoff = race.date でレース当日以降をリークさせない。
                // horse_stats/jockey_stats と同じく馬ごとの逐次クエリ（N+1）になるが、as_of/cutoff が
                // レース日ごとに変わる walk-forward の性質上バッチ化できないため、オフライン評価用途
                // として許容する（#30 で受容済みの方針と同じ）。
                let recent_form = self.recent_form_for(&r.horse_name, race.date).await?;
                let factors = build_factors(
                    &entry,
                    &course,
                    &horse,
                    jockey.as_ref(),
                    race.surface,
                    race.distance,
                    recent_form,
                );
                entry_factors.push((entry, factors));
            }

            let probs = paddock_domain::prediction::estimate_probabilities(&entry_factors);
            // entry_factors が非空なら estimate_probabilities は非空を返すため、理論上到達しない
            // 安全弁（空なら集計に寄与しないのでスキップ）。
            if probs.is_empty() {
                continue;
            }

            // 馬番 → (着順, 単勝オッズ) の突合表。entry_factors と同じ発走馬集合(starters)から作る。
            // 同着 1 着（稀）の場合は複数馬の win_outcomes が true になるが、的中率はトップ選好馬の
            // 着順のみで判定するため二重計上されない。Brier/LogLoss も破綻しない。
            let by_num: HashMap<u32, (Option<u32>, Option<f64>)> = starters
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
