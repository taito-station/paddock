//! 買い目（curated）の券種別 校正・回収率の集計（#121）と券種ラベルの出力順定数。

use std::collections::HashMap;

use super::metrics::calibration;
use super::model::{ExoticBet, ExoticSegment};

/// 買い目（curated）券種別セグメントの出力順（#121）。`BetCombination::type_label()` と一致させ、
/// select_bets の priority 順に準拠（馬連→馬単→三連複→単勝→複勝→三連単、ワイドは末尾）。
/// `wide` は現状 select_bets が生成しない（収支シミュレータ専用）ため exotic 集計では常に空＝
/// dead entry だが、将来ワイドを買い目に含めたときの取りこぼし防止と type_label 網羅のため残す。
pub(crate) const EXOTIC_BET_TYPES: [&str; 7] = [
    "quinella", "exacta", "trio", "win", "place", "trifecta", "wide",
];

/// 買い目（curated 推奨）を券種別に集計する（#121）。データのある券種のみ [`EXOTIC_BET_TYPES`] 順。
/// 各券種で 平均予測確率・実的中率・校正(Brier/LogLoss)・投票あたり回収率を出す。
pub fn exotic_segments(bets: &[ExoticBet]) -> Vec<ExoticSegment> {
    let mut buckets: HashMap<&'static str, Vec<&ExoticBet>> = HashMap::new();
    for b in bets {
        buckets.entry(b.bet_type).or_default().push(b);
    }

    EXOTIC_BET_TYPES
        .iter()
        .filter_map(|&label| {
            let group = buckets.get(label)?;
            let n = group.len() as f64;
            let pairs: Vec<(f64, bool)> = group.iter().map(|b| (b.predicted_prob, b.hit)).collect();
            let mean_predicted = group.iter().map(|b| b.predicted_prob).sum::<f64>() / n;
            let hits = group.iter().filter(|b| b.hit).count();
            let payout: f64 = group.iter().filter(|b| b.hit).map(|b| b.odds).sum();
            Some(ExoticSegment {
                label: label.to_string(),
                bets: group.len() as u32,
                mean_predicted,
                hit_rate: hits as f64 / n,
                calibration: calibration(&pairs),
                payout_rate: payout / n,
            })
        })
        .collect()
}
