//! 確率校正指標（Brier / LogLoss）と reliability 曲線の算出。

use super::model::{CalibrationMetrics, ReliabilityBin};

/// LogLoss で `ln(0)` を避けるための確率クランプ幅。`p` を `[EPS, 1-EPS]` に収める。
const LOG_LOSS_EPS: f64 = 1e-15;

/// reliability 曲線の等幅ビン数（`[0,0.1) … [0.9,1.0]`）。
pub(crate) const RELIABILITY_BINS: usize = 10;

/// `(予測確率, 実現したか)` のペア集合から Brier / LogLoss を算出する。空なら `ZERO`。
pub(crate) fn calibration(pairs: &[(f64, bool)]) -> CalibrationMetrics {
    if pairs.is_empty() {
        return CalibrationMetrics::ZERO;
    }
    let mut brier_sum = 0.0f64;
    let mut log_loss_sum = 0.0f64;
    for &(prob, hit) in pairs {
        let y = if hit { 1.0 } else { 0.0 };
        brier_sum += (prob - y).powi(2);
        // ε クランプで ln(0) の発散を防ぐ（スタッツ希薄で確率 0 の馬が実際に来るケース）。
        let p = prob.clamp(LOG_LOSS_EPS, 1.0 - LOG_LOSS_EPS);
        log_loss_sum += -(y * p.ln() + (1.0 - y) * (1.0 - p).ln());
    }
    let n = pairs.len() as f64;
    CalibrationMetrics {
        brier: brier_sum / n,
        log_loss: log_loss_sum / n,
    }
}

/// `(予測確率, 実現したか)` を等幅 `bins` ビンに分け、各ビンの平均予測と実測率を返す。
/// 確率は `[0,1]` にクランプし、上端 `1.0` は最終ビンに含める。空ビンも `count = 0` で返す。
pub(crate) fn reliability(pairs: &[(f64, bool)], bins: usize) -> Vec<ReliabilityBin> {
    debug_assert!(bins > 0, "reliability requires at least one bin");
    let width = 1.0 / bins as f64;
    let mut sum_pred = vec![0.0f64; bins];
    let mut hits = vec![0u32; bins];
    let mut counts = vec![0u32; bins];

    for &(prob, hit) in pairs {
        let p = prob.clamp(0.0, 1.0);
        // p == 1.0 を最終ビンへ。境界値（例 0.3）は浮動小数点誤差で隣接ビンに入りうるが、
        // reliability の概観用途では許容する。
        let idx = ((p / width) as usize).min(bins - 1);
        sum_pred[idx] += p;
        if hit {
            hits[idx] += 1;
        }
        counts[idx] += 1;
    }

    (0..bins)
        .map(|i| {
            let count = counts[i];
            let (mean_predicted, observed_rate) = if count > 0 {
                (sum_pred[i] / count as f64, hits[i] as f64 / count as f64)
            } else {
                (0.0, 0.0)
            };
            ReliabilityBin {
                lower: i as f64 * width,
                upper: (i + 1) as f64 * width,
                count,
                mean_predicted,
                observed_rate,
            }
        })
        .collect()
}
