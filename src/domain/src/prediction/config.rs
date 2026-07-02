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
    /// 脚質（先行度）項の重みオーバーライド（#329 Phase1）。`None` のとき `weights::RUNNING_STYLE_WEIGHT`
    /// （0.0）を使う。backtest の `--running-style-weight` スイープ専用（measure-first）。predict 本番は `None`。
    pub running_style_weight: Option<f64>,
    /// 騎手直近フォーム項の重みオーバーライド（#221）。`None` のとき `weights::JOCKEY_RECENT_FORM_WEIGHT`
    /// を使う。backtest の `--jockey-form-weight` スイープ専用（ADR 0038）。predict 本番は `None`。
    pub jockey_recent_form_weight: Option<f64>,
    /// win_prob 冪変換 `win'_i ∝ win_i^gamma` のγ（#246）。`None` のとき no-op（後方互換）。
    /// `gamma > 1.0` で人気馬の win を相対強調し穴の 1 着過大評価を縮約する。ブレンド後の最終 win に
    /// 適用する（[`super::estimate::apply_win_power`]）。backtest の `--win-power` スイープ専用で、
    /// 採用値は backtest 検証後に `production()` へ反映する（ADR 0042）。
    pub win_power: Option<f64>,
    /// place/show スコアの冪変換 γ（#283 / #258 Phase 2）。`None` のとき no-op（後方互換）。
    /// `normalize_to_sum(score^γ, target)` で正規化前にスコアをシャープ化し、正規化＋単調化が招く
    /// 分布の中央圧縮（本命の複勝を過小評価・人気薄を過大評価）を脱圧縮する。`γ > 1.0` で本命の
    /// place/show を持ち上げ人気薄を下げる（win の [`super::estimate::apply_win_power`] と同型だが、
    /// place/show は推定時にスコアへ適用し場内合計 2.0/3.0 を保つ点が異なる）。`production()` は
    /// 採用値 `RECOMMENDED_PLACE_SHOW_POWER`（ADR 0047）を既定にする。再 sweep は backtest の
    /// `--place-show-power` フラグ経由（`Default` は `None`＝no-op）。
    pub place_show_power: Option<f64>,
    /// 欠落 stat factor をレース内 field mean で補完するか（#272 改善② / ADR 0057）。`false`（`Default`）は
    /// 従来どおり欠落項を母数から落とす（drop）。`true` は present 馬の縮約後レート平均（present<2 は prior）を
    /// 欠く馬に代入して weight も数える。欠落を drop すると識別力の高い高欠落 factor
    /// （horse_surface/distance/track_condition, 欠落 0.28〜0.39）の resolution が希釈されるため、field mean
    /// で present 馬の相対差を保ったまま欠く馬を中立に置く。診断ダンプ screening で純 AUC 0.671→0.678・
    /// top1 0.182→0.197（全 6 四半期改善）。`production()` は `true`、backtest の `--impute-missing-factors`
    /// で A/B できる。詳細は [`super::scoring::FactorImpute`] と ADR 0057。
    pub impute_missing_factors: bool,
}

// trend_n のデフォルト値が 0 でなく 1 のため、derive(Default) ではなく手書き impl を使う。
impl Default for EstimationConfig {
    fn default() -> Self {
        Self {
            shrinkage: None,
            recency: None,
            recent_form_weight: None,
            trend_n: 1,
            running_style_weight: None,
            jockey_recent_form_weight: None,
            win_power: None,
            place_show_power: None,
            impute_missing_factors: false,
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

/// 本番 predict が採用する place/show スコア冪変換のγ（#283 / #258 Phase 2）。backtest
/// （2025-01-01〜2026-06-27 / 4891R, α=0.2・m=10・win_power=1.25）の**当初範囲 γ∈{none,1.25,1.5,2.0}**
/// では γ が大きいほど place/show Brier・LogLoss・人気帯校正・複勝買い目 ROI が単調改善、単勝校正は
/// 全γで完全不変（place/show のみ冪変換し win を触らない設計）。当初範囲では γ=2.0 が最良
/// （show Brier 0.1492→0.1461、複勝買い目 ROI 76.7→79.2%、1番人気の複勝過小評価 +24.8→+22.9pt）。
/// 本命ギャップは +22.9pt とまだ大きく過補正の手前で、安全側。γ≥2.5 は #290 で延長 sweep 済み
/// （#286/ADR 0050 で raw 素スコアが不変確定したため現素スコア上で掃引できた）。純校正の knee は
/// γ=3.0 だが複勝買い目 ROI は γ を上げて net 改善せず（むしろ 3.0 で劣化）、favorite の複勝過小評価も
/// 構造的に埋まらないため、トレードオフ上 γ=2.0 維持が妥当と確定した。詳細は ADR 0047 / 0050 / 0051。
pub const RECOMMENDED_PLACE_SHOW_POWER: f64 = 2.0;

/// 本番 predict が採用する市場オッズ(単勝)ブレンドのモデル重み α（#72）。`None` はモデルのみ。
/// backtest（2025-01〜2026-06 / 4891R）の α スイープで Brier/LogLoss が α=0.2 で最良（ADR 0034）。
/// 市場オッズが無いレースは自動でモデルのみにフォールバックする。
/// 詳細は docs/specifications/probability-estimation.md。
pub const RECOMMENDED_MARKET_BLEND_ALPHA: Option<f64> = Some(0.2);

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
            // #329 Phase1: 脚質は measure-first で production 非組込（重み 0）。lift 判定後に採用値を入れる。
            running_style_weight: None,
            trend_n: 1, // #220 backtest にて N=2/3 は全指標悪化のため棄却（ADR-0036）
            jockey_recent_form_weight: None, // #221 暫定 weight（const）を使用。sweep は ADR 0038
            // #246: γ=1.25 を採用（4891R sweep で単勝 LogLoss 0.1974→0.1954 最良・穴帯校正改善、
            // γ≥1.5 は LogLoss/Brier 悪化＋人気馬過剰補正で棄却）。詳細は ADR 0042。
            win_power: Some(RECOMMENDED_WIN_POWER),
            // #283 Phase 2: γ=2.0 を採用（4891R sweep で place/show Brier/LogLoss・人気帯校正・
            // 複勝 ROI が単調改善・単勝校正は完全不変）。詳細は ADR 0047。
            place_show_power: Some(RECOMMENDED_PLACE_SHOW_POWER),
            // #272 改善②: 欠落 stat factor を field mean 補完（純 AUC 0.671→0.678・top1 0.182→0.197,
            // 全 6 四半期改善／blended α=0.2 非回帰）。詳細は ADR 0057。
            impute_missing_factors: true,
        }
    }
}
