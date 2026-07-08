//! レース形状の decision-support 指標（#344）。ライブ日次ボードの「段階ROI」と「荒れ度」を、
//! 買い妙味（期待値）と分布の乱れ（変動）という**別軸**として表す。いずれも純関数で、
//! スコア経路・確率推定には一切影響しない（表示のみ・ADR0055/0060 の decision-support）。

/// 荒れ度（分布の乱れ）を表すラベルの境界。純モデル勝率の正規化エントロピー `[0,1]` に対する
/// しきい値（調整可）。`< SOLID` を堅い、`>= WIDE_OPEN` を荒れ、間を標準とする。
const ROUGHNESS_SOLID: f64 = 0.60;
const ROUGHNESS_WIDE_OPEN: f64 = 0.85;

/// 純モデル勝率分布の「荒れ度」を正規化シャノンエントロピーで返す（#344）。
///
/// `H = -Σ pᵢ·ln pᵢ / ln N` を `[0,1]` に正規化する。1 に近いほど拮抗（荒れ）、0 に近いほど
/// 一頭に確率が集中（堅い）。**純モデル勝率（α=1.0・市場非ブレンド）** を渡す前提で、オッズに
/// 依存しないレース形状の指標にする（ライブでも過去照合でも同じ値になる）。
///
/// - 入力は勝率（合計 1 前提だが、数値誤差に強いよう内部で正規化する）。
/// - 頭数 N<2、または合計が 0 のときは荒れ度を定義できないため `0.0` を返す。
/// - 0 以下の確率（欠損・負の数値誤差）は寄与 0 として無視し、**正の確率を持つ頭数**で正規化する
///   （`ln(n)`）。純モデルは全頭に正の勝率を与えるため実運用で n は出走頭数と一致するが、万一
///   厳密 0 が混じると母数 n が縮み荒れ度がやや過大化しうる（decision-support 表示のため許容）。
pub fn race_roughness(win_probs: &[f64]) -> f64 {
    let total: f64 = win_probs.iter().filter(|p| **p > 0.0).sum();
    let n = win_probs.iter().filter(|p| **p > 0.0).count();
    if n < 2 || total <= 0.0 {
        return 0.0;
    }
    let mut entropy = 0.0;
    for &p in win_probs.iter().filter(|p| **p > 0.0) {
        let q = p / total;
        entropy -= q * q.ln();
    }
    // ln(n) で正規化（一様分布のとき 1.0）。n>=2 なので ln(n)>0。
    (entropy / (n as f64).ln()).clamp(0.0, 1.0)
}

/// 荒れ度スコアを日本語ラベルへ（表示用）。
pub fn roughness_label(score: f64) -> &'static str {
    if score >= ROUGHNESS_WIDE_OPEN {
        "荒れ"
    } else if score < ROUGHNESS_SOLID {
        "堅い"
    } else {
        "標準"
    }
}

/// 段階ROI の tier（買い強度）。ライブ日次ボードの常時ランキング表示に使う（#344）。
/// 「買い」は ROI≥100（+EV）のみ。それ未満は当日の ROI 分布に対する相対位置で 3 段に分け、
/// 常に在庫（上位 2/3）が出るようにする。**-EV を「買い」に見せない**（ADR0055/0060・軸ロック）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageTier {
    /// 🟢 買い（ROI ≥ 100・+EV）
    Buy,
    /// 🟡 惜しい（ROI < 100 かつ当日分布の上位＝ p66 以上）
    Close,
    /// ⚪ 様子見（当日分布の中位＝ p33 以上 p66 未満）
    Watch,
    /// 非表示（当日分布の下位＝ p33 未満）
    Hidden,
}

impl StageTier {
    /// SPA/REST で運ぶ安定スラッグ。表示用のバッジ文字列は SPA 側（`web/src/lib/live.ts` の
    /// `TIER_BADGE`）が slug から生成するため、Rust 側はスラッグのみを提供して二重管理を避ける。
    pub fn as_str(&self) -> &'static str {
        match self {
            StageTier::Buy => "buy",
            StageTier::Close => "close",
            StageTier::Watch => "watch",
            StageTier::Hidden => "hidden",
        }
    }
}

/// ROI[%] と当日全レースの ROI[%] 群から段階 tier を決める（#344・純関数）。
///
/// - `roi_pct >= 100` → `Buy`（絶対閾値・+EV のみ）。
/// - それ未満は「当日の ROI<100 のレース群」に対する分位で分ける。上位 1/3（p66 以上）を `Close`、
///   中位（p33 以上）を `Watch`、下位（p33 未満）を `Hidden`。これで固定 90 floor による「毎日空」を
///   避け、常に上位 2/3 の在庫を出す（ADR0040 の ROI 分布が 75–85 に偏る前提への対処）。
/// - 分位の母集団は `day_rois` のうち 100 未満のもの。母集団が空（全レース +EV 等）や当該が Buy の
///   ときは分位を使わない。同値だらけで p33==p66 のときは境界を含める側（>=）で Watch 以上に寄せる。
pub fn stage_tier(roi_pct: f64, day_rois: &[f64]) -> StageTier {
    if roi_pct >= 100.0 {
        return StageTier::Buy;
    }
    // 当日 ROI<100 の母集団で相対位置を測る。
    let mut sub: Vec<f64> = day_rois.iter().copied().filter(|r| *r < 100.0).collect();
    if sub.len() < 3 {
        // 母集団が小さすぎて分位が意味を持たない → 全て Watch（在庫は出すが強調しない）。
        return StageTier::Watch;
    }
    sub.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p33 = percentile(&sub, 1.0 / 3.0);
    let p66 = percentile(&sub, 2.0 / 3.0);
    if roi_pct >= p66 {
        StageTier::Close
    } else if roi_pct >= p33 {
        StageTier::Watch
    } else {
        StageTier::Hidden
    }
}

/// 昇順ソート済みスライスの分位点（線形補間なしの nearest-rank 風）。`q∈[0,1]`。
fn percentile(sorted_asc: &[f64], q: f64) -> f64 {
    debug_assert!(!sorted_asc.is_empty());
    let idx = (q * (sorted_asc.len() as f64 - 1.0)).round() as usize;
    sorted_asc[idx.min(sorted_asc.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roughness_uniform_is_one() {
        // 一様分布（総流れ）は正規化エントロピー 1.0。
        let probs = vec![0.25, 0.25, 0.25, 0.25];
        assert!((race_roughness(&probs) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn roughness_dominant_favorite_is_low() {
        // 断然人気（一頭に集中）は荒れ度が低い。
        let chalk = race_roughness(&[0.9, 0.05, 0.03, 0.02]);
        let open = race_roughness(&[0.3, 0.3, 0.2, 0.2]);
        assert!(chalk < open, "断然人気 {chalk} < 拮抗 {open}");
        assert!(chalk < ROUGHNESS_SOLID, "断然人気は堅い判定");
    }

    #[test]
    fn roughness_handles_degenerate_inputs() {
        assert_eq!(race_roughness(&[]), 0.0);
        assert_eq!(race_roughness(&[1.0]), 0.0, "1 頭は荒れ度未定義");
        assert_eq!(race_roughness(&[0.0, 0.0]), 0.0, "総和 0 は未定義");
        // 負値・0 は寄与 0 として無視し、残りで計算する。
        let r = race_roughness(&[0.5, 0.5, -0.1, 0.0]);
        assert!((r - 1.0).abs() < 1e-9);
    }

    #[test]
    fn roughness_label_bands() {
        assert_eq!(roughness_label(0.90), "荒れ");
        assert_eq!(roughness_label(0.70), "標準");
        assert_eq!(roughness_label(0.50), "堅い");
        // 境界: SOLID は標準側、WIDE_OPEN は荒れ側。
        assert_eq!(roughness_label(ROUGHNESS_SOLID), "標準");
        assert_eq!(roughness_label(ROUGHNESS_WIDE_OPEN), "荒れ");
    }

    #[test]
    fn stage_tier_buy_is_absolute_100() {
        assert_eq!(stage_tier(100.0, &[100.0, 80.0, 70.0]), StageTier::Buy);
        assert_eq!(stage_tier(120.0, &[]), StageTier::Buy);
        // 99% は +EV でないので Buy にしない（-EV/微負を買いに見せない）。
        assert_ne!(stage_tier(99.9, &[80.0, 70.0, 60.0, 99.9]), StageTier::Buy);
    }

    #[test]
    fn stage_tier_relative_bands_have_inventory() {
        // 当日 6 レース（全て <100）。上位 2/3 は Close/Watch で在庫、下位 1/3 が Hidden。
        let day = vec![85.0, 82.0, 80.0, 78.0, 75.0, 70.0];
        assert_eq!(stage_tier(85.0, &day), StageTier::Close, "上位は惜しい");
        assert_eq!(stage_tier(78.0, &day), StageTier::Watch, "中位は様子見");
        assert_eq!(stage_tier(70.0, &day), StageTier::Hidden, "下位は非表示");
    }

    #[test]
    fn stage_tier_small_sample_defaults_to_watch() {
        // 母集団<3 は分位が無意味 → Watch（在庫は出す）。
        assert_eq!(stage_tier(80.0, &[80.0, 70.0]), StageTier::Watch);
    }

    #[test]
    fn stage_tier_all_equal_does_not_hide_everything() {
        // 全て同値なら p33==p66 で、境界含めにより Hidden に落とさない（在庫を確保）。
        let day = vec![80.0, 80.0, 80.0, 80.0];
        assert_ne!(stage_tier(80.0, &day), StageTier::Hidden);
    }
}
