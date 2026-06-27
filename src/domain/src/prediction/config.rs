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

/// 確率推定の挙動切替（#75/#217/#220）。`Default` は後方互換（縮約・減衰なし / デフォルト重み / 直近 1 走）。
/// backtest が CLI から組み立てて before/after を比較し、採用値を predict のデフォルトに反映する。
#[derive(Debug, Clone, Copy)]
pub struct EstimationConfig {
    pub shrinkage: Option<ShrinkageConfig>,
    pub recency: Option<RecencyConfig>,
    /// 前走フォーム項の重みオーバーライド（#217）。`None` のとき `weights::FORM_WEIGHT`（0.25）を使う。
    /// backtest の `--recent-form-weight` スイープ専用。predict 本番経路は `None`（デフォルト重み）。
    pub recent_form_weight: Option<f64>,
    /// 直近 N 走トレンドの走数（#220）。重みは [1.0, 0.5, 0.25] 固定。
    /// `1` = 前走のみ（現行挙動）、`2`/`3` = 加重平均。
    pub trend_n: u32,
    /// 騎手直近フォーム項の重みオーバーライド（#221）。`None` のとき `weights::JOCKEY_RECENT_FORM_WEIGHT`
    /// を使う。backtest の `--jockey-form-weight` スイープ専用（ADR 0038）。predict 本番は `None`。
    pub jockey_recent_form_weight: Option<f64>,
    /// win_prob 冪変換 `win'_i ∝ win_i^gamma` のγ（#246）。`None` のとき no-op（後方互換）。
    /// `gamma > 1.0` で人気馬の win を相対強調し穴の 1 着過大評価を縮約する。ブレンド後の最終 win に
    /// 適用する（[`super::estimate::apply_win_power`]）。backtest の `--win-power` スイープ専用で、
    /// 採用値は backtest 検証後に `production()` へ反映する（ADR 0042）。
    pub win_power: Option<f64>,
}

// trend_n のデフォルト値が 0 でなく 1 のため、derive(Default) ではなく手書き impl を使う。
impl Default for EstimationConfig {
    fn default() -> Self {
        Self {
            shrinkage: None,
            recency: None,
            recent_form_weight: None,
            trend_n: 1,
            jockey_recent_form_weight: None,
            win_power: None,
        }
    }
}

/// 本番 predict が採用するベイズ縮約の擬似カウント（#75）。backtest（2026-03-28〜05-31 / 144R,
/// #81 後ロジック）で m∈{off,5,10,20,50} を比較し、m=10 が単勝 Brier/LogLoss・連対で最良、
/// 的中率も改善（off 比 単勝 LogLoss 0.272→0.251、単勝的中 9.7→13.2%）だったため採用。
/// m=50 は過縮約で劣化。
pub const RECOMMENDED_SHRINKAGE_M: f64 = 10.0;

/// 本番 predict が採用する win_prob 冪変換のγ（#246）。backtest（2025-01-01〜2026-06-30 / 4891R,
/// α=0.2・m=10）で γ∈{1.0,1.25,1.5,2.0} を比較し、γ=1.25 が単勝 LogLoss 最良（0.1974→0.1954）で
/// 穴帯（7〜9番人気・10番人気以下）の 1 着過大評価を縮小、トップ選好・回収率は単調変換のため不変。
/// γ≥1.5 は LogLoss/Brier 悪化＋人気馬を過剰補正（1番人気 予測 37.5%/46.7% vs 実測 28.2%）のため棄却。
/// 詳細は ADR 0042。
pub const RECOMMENDED_WIN_POWER: f64 = 1.25;

impl EstimationConfig {
    /// 本番 predict 経路のデフォルト設定（#75 で backtest 検証して採用した値）。
    /// backtest の `--shrinkage-m` 未指定（= `Default`, 縮約 off）とは別で、こちらは縮約 on。
    pub fn production() -> Self {
        Self {
            shrinkage: Some(ShrinkageConfig {
                pseudo_count: RECOMMENDED_SHRINKAGE_M,
            }),
            recency: None,
            recent_form_weight: None,
            trend_n: 1, // #220 backtest にて N=2/3 は全指標悪化のため棄却（ADR-0036）
            jockey_recent_form_weight: None, // #221 暫定 weight（const）を使用。sweep は ADR 0038
            // #246: γ=1.25 を採用（4891R sweep で単勝 LogLoss 0.1974→0.1954 最良・穴帯校正改善、
            // γ≥1.5 は LogLoss/Brier 悪化＋人気馬過剰補正で棄却）。詳細は ADR 0042。
            win_power: Some(RECOMMENDED_WIN_POWER),
        }
    }
}
