//! Harville モデルによる連系・順序系券種の的中確率推定（単勝確率からの導出）。

/// Harville の分母が 0 付近に潰れたときのクランプ下限（ゼロ除算回避）。
const MIN_DENOMINATOR: f64 = 1e-6;

/// P(a→b): Harville conditional probability that b finishes 2nd given a wins.
///
/// Returns `0.0` when `win_a >= 1.0` (denominator `1 - win_a` would be zero or negative).
/// Unlike `harville_trifecta`, the guard here only checks `win_a` because `win_b`
/// does not appear in the denominator.
pub(crate) fn harville_exacta(win_a: f64, win_b: f64) -> f64 {
    if win_a >= 1.0 {
        return 0.0;
    }
    let denom = (1.0 - win_a).max(MIN_DENOMINATOR);
    win_a * win_b / denom
}

/// P(quinella {a,b}) = P(a→b) + P(b→a).
pub(crate) fn harville_quinella(win_a: f64, win_b: f64) -> f64 {
    harville_exacta(win_a, win_b) + harville_exacta(win_b, win_a)
}

/// P(trifecta a→b→c): Harville sequential conditional probability.
///
/// Precondition: `win_a + win_b < 1.0`. Returns `0.0` when this is violated
/// to avoid a negative denominator being clamped to MIN_DENOMINATOR, which
/// would produce an unrealistically large probability.
pub(crate) fn harville_trifecta(win_a: f64, win_b: f64, win_c: f64) -> f64 {
    if win_a + win_b >= 1.0 {
        return 0.0;
    }
    let denom_a = (1.0 - win_a).max(MIN_DENOMINATOR);
    // The guard ensures 1-win_a-win_b > 0, but min-clamp is kept for floating-point safety
    // when win_a+win_b is very close to 1.0.
    let denom_ab = (1.0 - win_a - win_b).max(MIN_DENOMINATOR);
    win_a * (win_b / denom_a) * (win_c / denom_ab)
}

/// P(trio {a,b,c}) = sum of all 6 ordered permutations as trifecta probabilities.
pub(crate) fn harville_trio(win_a: f64, win_b: f64, win_c: f64) -> f64 {
    harville_trifecta(win_a, win_b, win_c)
        + harville_trifecta(win_a, win_c, win_b)
        + harville_trifecta(win_b, win_a, win_c)
        + harville_trifecta(win_b, win_c, win_a)
        + harville_trifecta(win_c, win_a, win_b)
        + harville_trifecta(win_c, win_b, win_a)
}
