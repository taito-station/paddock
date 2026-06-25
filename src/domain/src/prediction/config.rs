//! 確率推定の挙動切替設定（ベイズ縮約・リーセンシー, #75）。

/// ベイズ縮約（shrinkage, #75）の設定。出走数 `k` が少ない factor のレートを母集団平均
/// `PRIOR_RATE` へ `smoothed = (k·rate + m·prior)/(k + m)` で寄せ、少データ馬の過信
/// （`win_prob=0` を含む, ADR 0002）を緩和する。`pseudo_count = m` は擬似標本数。
#[derive(Debug, Clone, Copy)]
pub struct ShrinkageConfig {
    pub pseudo_count: f64,
}

/// リーセンシー重み付け（recency, #75）の設定。直近成績に時間減衰
/// `w = 0.5^(days_ago/half_life)` を掛けて集計する（Phase B で使用）。
#[derive(Debug, Clone, Copy)]
pub struct RecencyConfig {
    pub half_life_days: f64,
}

/// 確率推定の挙動切替（#75, #220）。`Default` は後方互換（縮約・減衰なし / 直近 1 走）。
/// backtest が CLI から組み立てて before/after を比較し、採用値を predict のデフォルトに反映する。
#[derive(Debug, Clone, Copy)]
pub struct EstimationConfig {
    pub shrinkage: Option<ShrinkageConfig>,
    pub recency: Option<RecencyConfig>,
    /// 直近 N 走トレンドの走数（#220）。重みは [1.0, 0.5, 0.25] 固定。
    /// `1` = 前走のみ（現行挙動）、`2`/`3` = 加重平均。
    pub trend_n: u32,
}

// trend_n のデフォルト値が 0 でなく 1 のため、derive(Default) ではなく手書き impl を使う。
impl Default for EstimationConfig {
    fn default() -> Self {
        Self {
            shrinkage: None,
            recency: None,
            trend_n: 1,
        }
    }
}

/// 本番 predict が採用するベイズ縮約の擬似カウント（#75）。backtest（2026-03-28〜05-31 / 144R,
/// #81 後ロジック）で m∈{off,5,10,20,50} を比較し、m=10 が単勝 Brier/LogLoss・連対で最良、
/// 的中率も改善（off 比 単勝 LogLoss 0.272→0.251、単勝的中 9.7→13.2%）だったため採用。
/// m=50 は過縮約で劣化。
pub const RECOMMENDED_SHRINKAGE_M: f64 = 10.0;

impl EstimationConfig {
    /// 本番 predict 経路のデフォルト設定（#75 で backtest 検証して採用した値）。
    /// backtest の `--shrinkage-m` 未指定（= `Default`, 縮約 off）とは別で、こちらは縮約 on。
    pub fn production() -> Self {
        Self {
            shrinkage: Some(ShrinkageConfig {
                pseudo_count: RECOMMENDED_SHRINKAGE_M,
            }),
            recency: None,
            trend_n: 1, // #220 backtest にて N=2/3 は全指標悪化のため棄却（ADR-0035）
        }
    }
}
