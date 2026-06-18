//! Kelly 基準による賭け額分数（上限クランプ付き）。

/// Kelly fraction with cap: f = (p*b - q) / b, clamped to [0, kelly_cap].
///
/// `gross_odds` is the JRA payout multiplier (e.g. 3.5 means ¥350 back on ¥100).
/// Net odds b = gross_odds - 1.0 (gross → net 変換). EV = p * gross_odds; EV > 1.0 が期待値プラス。
/// Returns `0.0` when `gross_odds <= 1.0` (no net profit possible, avoids zero division).
pub(crate) fn kelly_fraction(p: f64, gross_odds: f64, kelly_cap: f64) -> f64 {
    let b = gross_odds - 1.0;
    if b <= 0.0 {
        return 0.0;
    }
    let q = 1.0 - p;
    let f = (p * b - q) / b;
    f.max(0.0).min(kelly_cap)
}
