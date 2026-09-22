//! Harville モデルによる連系・順序系券種の的中確率推定（単勝確率からの導出）。
//!
//! 素の Harville（`harville_*` 自由関数）に加え、Lo & Bacon-Shone の discounted Harville
//! （2 着段 λ2 / 3 着段 λ3 の冪割引・#703 Phase 2）を [`HarvilleModel`] が提供する。
//! 素の Harville は強い馬の 2・3 着確率を系統的に過大評価する（Benter 1994 Table 9/10:
//! Z=−4.3〜−8.3。JRA 44,774R の伊藤 2010 で λ2=0.81/λ3=0.70 が対数尤度 1,500 差で優る。
//! 一次資料: docs/docs-original/703-probability-logic-literature-survey.md §2）。

use std::collections::HashMap;

use crate::horse_result::HorseNum;

/// Harville の分母が 0 付近に潰れたときのクランプ下限（ゼロ除算回避）。
const MIN_DENOMINATOR: f64 = 1e-6;

/// discounted Harville の割引指数（Lo & Bacon-Shone / Benter 形）。
///
/// 2 着段用 σ_i ∝ w_i^λ2、3 着段用 τ_i ∝ w_i^λ3（いずれも場内正規化）を作り、
/// P(a→b→c) = w_a · σ_b/(1−σ_a) · τ_c/(1−τ_a−τ_b) とする。λ2=λ3=1.0 は素の Harville と
/// **bit-exact に一致**する（[`HarvilleModel`] が自由関数へ委譲する）。λ<1 ほど
/// 「2・3 着争いはより運任せ」＝強い馬の連対・複勝圏確率が下がる。
///
/// 事前値の目安: 伊藤 2010（JRA）λ2=0.81/λ3=0.70、Benter（香港）0.81/0.65。採用値は
/// fit 窓 MLE → eval 窓ゲート（backtest.md 評価プロトコル）を通してから定数に反映する。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HarvilleParams {
    /// 2 着段の割引指数 λ2（有限かつ > 0。1.0 = 無割引）。
    pub lambda2: f64,
    /// 3 着段の割引指数 λ3（有限かつ > 0。1.0 = 無割引）。
    pub lambda3: f64,
}

impl HarvilleParams {
    /// 素の Harville と厳密一致する無割引パラメータ。
    pub const IDENTITY: Self = Self {
        lambda2: 1.0,
        lambda3: 1.0,
    };

    /// λ の実用上限。fit スクリプト（harville_lambda_fit.py）の探索域 0.2〜6.0 を余裕を持って
    /// 包む値で、これを超える λ は測定の裏付けを持ち得ず割引として意味がない（入力ミスとして
    /// 弾く。w は [0,1] にクランプ済みのため数値破綻の防止ではなく妥当性の境界）。
    pub const MAX_LAMBDA: f64 = 10.0;

    /// 検証付きコンストラクタ。非有限・0 以下・[`Self::MAX_LAMBDA`] 超の λ は割引として意味を
    /// 持たないため `None` を返す（呼び出し側で入力エラーにする。黙って IDENTITY へ倒さない）。
    pub fn new(lambda2: f64, lambda3: f64) -> Option<Self> {
        let ok = |l: f64| l.is_finite() && l > 0.0 && l <= Self::MAX_LAMBDA;
        if ok(lambda2) && ok(lambda3) {
            Some(Self { lambda2, lambda3 })
        } else {
            None
        }
    }

    fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

impl Default for HarvilleParams {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// EV 経路（portfolio・ev_probs = 純モデル α=1.0）用の推奨割引。**IDENTITY 維持**（#703
/// Phase 2 の採用ゲートで棄却）。pure 系統の fit 窓 MLE は λ2=2.29 [2.13,2.42] /
/// λ3=1.80 [1.65,1.94] と**文献（伊藤 2010: 0.81/0.70）と逆方向**（縮約 m=10 で平坦化した
/// 純モデルは条件付き段も平坦すぎるため）で、ゲート (c)（文献事前値との整合）を満たさない。
/// 実測の詳細と再検討条件は betting-rule-history.md 決定ログ #703 参照。
pub const RECOMMENDED_HARVILLE_LAMBDA_PURE: HarvilleParams = HarvilleParams::IDENTITY;

/// **blended 確率**に対する推奨割引。**λ2=0.90 / λ3=0.77 採用**（#703 Phase 2）。
/// fit 窓（2025 年・2,971R）MLE: λ2=0.90 [0.87,0.93] / λ3=0.77 [0.74,0.80]
/// ——伊藤 2010（JRA 44,774R: 0.81/0.70）・Benter（香港: 0.81/0.65）と同方向・同水準。
/// eval 窓（2026-01〜08）で券種別校正が全面改善（馬連 Brier 0.0684→0.0585・馬単/3連単の
/// 平均予測と実的中率が一致）。**適用は blended 確率を渡す呼び出し側の責務**
/// （analyze backtest の `--blend-alpha` 指定時が該当。`BettingConfig::default()` は IDENTITY
/// のまま＝pure 確率への誤適用を防ぐ）。決定の経緯は betting-rule-history.md 決定ログ #703。
pub const RECOMMENDED_HARVILLE_LAMBDA_BLENDED: HarvilleParams = HarvilleParams {
    lambda2: 0.90,
    lambda3: 0.77,
};

/// 場全体の単勝確率から σ（2 着段）・τ（3 着段）を前計算した discounted Harville 評価器。
///
/// σ/τ は**場内正規化**を要するためペア単位の自由関数では表現できない（#703 Phase 2）。
/// `params == IDENTITY` のときは全メソッドが素の Harville 自由関数へ委譲し bit-exact に
/// 一致する（λ=1 でも正規化パスを通すと Σw≠1 の入力で素の Harville とズレるため）。
/// モデルに無い馬番の参照は確率 0 として扱う（`simulate` の `win_of` と同じ規約）。
pub(crate) struct HarvilleModel {
    /// 馬番 → (w, σ, τ)。IDENTITY のときは σ=τ=w（委譲パスでは参照されない）。
    entries: HashMap<HorseNum, (f64, f64, f64)>,
    identity: bool,
}

impl HarvilleModel {
    pub(crate) fn new(
        win_probs: impl IntoIterator<Item = (HorseNum, f64)>,
        params: HarvilleParams,
    ) -> Self {
        let mut raw: Vec<(HorseNum, f64)> = win_probs.into_iter().collect();
        // σ/τ の正規化和は f64 加算の順序に依存するため馬番順に固定する。HashMap 由来の
        // 入力（simulate 経路）でも run 間で結果が bit 単位で揺れないようにする（決定性の担保）。
        raw.sort_by_key(|(h, _)| h.value());
        if params.is_identity() {
            return Self {
                entries: raw.into_iter().map(|(h, w)| (h, (w, w, w))).collect(),
                identity: true,
            };
        }
        // 値域 [0,1] への防御クランプ。負値・非有限は 0、1 超は 1 に潰す（確率としては不正な
        // 入力だが、w>1 × 大 λ の powf が inf → σ=NaN となって縮退ガードをすり抜けるのを防ぐ）。
        let clamp = |w: f64| {
            if w.is_finite() {
                w.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let s2: f64 = raw
            .iter()
            .map(|(_, w)| clamp(*w).powf(params.lambda2))
            .sum();
        let s3: f64 = raw
            .iter()
            .map(|(_, w)| clamp(*w).powf(params.lambda3))
            .sum();
        let entries = raw
            .into_iter()
            .map(|(h, w)| {
                let w = clamp(w);
                let sigma = if s2 > 0.0 {
                    w.powf(params.lambda2) / s2
                } else {
                    0.0
                };
                let tau = if s3 > 0.0 {
                    w.powf(params.lambda3) / s3
                } else {
                    0.0
                };
                (h, (w, sigma, tau))
            })
            .collect();
        Self {
            entries,
            identity: false,
        }
    }

    /// 馬番の (w, σ, τ)。モデルに無い馬は全成分 0（= その馬絡みの組合せ確率 0）。
    fn of(&self, h: HorseNum) -> (f64, f64, f64) {
        self.entries.get(&h).copied().unwrap_or((0.0, 0.0, 0.0))
    }

    /// P(a→b)（馬単）: w_a · σ_b / (1−σ_a)。
    pub(crate) fn exacta(&self, a: HorseNum, b: HorseNum) -> f64 {
        let (wa, sa, _) = self.of(a);
        let (_, sb, _) = self.of(b);
        if self.identity {
            return harville_exacta(wa, sb);
        }
        if sa >= 1.0 {
            return 0.0;
        }
        wa * sb / (1.0 - sa).max(MIN_DENOMINATOR)
    }

    /// P(馬連 {a,b}) = P(a→b) + P(b→a)。
    pub(crate) fn quinella(&self, a: HorseNum, b: HorseNum) -> f64 {
        self.exacta(a, b) + self.exacta(b, a)
    }

    /// P(a→b→c)（三連単）: w_a · σ_b/(1−σ_a) · τ_c/(1−τ_a−τ_b)。
    pub(crate) fn trifecta(&self, a: HorseNum, b: HorseNum, c: HorseNum) -> f64 {
        let (wa, sa, ta) = self.of(a);
        let (_, sb, tb) = self.of(b);
        let (_, _, tc) = self.of(c);
        if self.identity {
            // identity では entries が (w,w,w) なので sb=w_b / tc=w_c ＝素の式と同引数。
            return harville_trifecta(wa, sb, tc);
        }
        // 素の Harville と同じ縮退ガード（負の分母をクランプで巨大確率化させない）を
        // 各段の正規化配列に対して掛ける。
        if sa >= 1.0 || ta + tb >= 1.0 {
            return 0.0;
        }
        let denom_a = (1.0 - sa).max(MIN_DENOMINATOR);
        let denom_ab = (1.0 - ta - tb).max(MIN_DENOMINATOR);
        wa * (sb / denom_a) * (tc / denom_ab)
    }

    /// P(三連複 {a,b,c}) = 6 順列の trifecta 和。
    pub(crate) fn trio(&self, a: HorseNum, b: HorseNum, c: HorseNum) -> f64 {
        self.trifecta(a, b, c)
            + self.trifecta(a, c, b)
            + self.trifecta(b, a, c)
            + self.trifecta(b, c, a)
            + self.trifecta(c, a, b)
            + self.trifecta(c, b, a)
    }
}

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
///
/// 本体経路は [`HarvilleModel::quinella`]（IDENTITY で本関数と厳密一致）に移行済み。
/// 素の Harville の正本・恒等テストの参照実装として保持する。
#[cfg_attr(not(test), allow(dead_code))]
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
///
/// 本体経路は [`HarvilleModel::trio`]（IDENTITY で本関数と厳密一致）に移行済み。
/// 素の Harville の正本・恒等テストの参照実装として保持する。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn harville_trio(win_a: f64, win_b: f64, win_c: f64) -> f64 {
    harville_trifecta(win_a, win_b, win_c)
        + harville_trifecta(win_a, win_c, win_b)
        + harville_trifecta(win_b, win_a, win_c)
        + harville_trifecta(win_b, win_c, win_a)
        + harville_trifecta(win_c, win_a, win_b)
        + harville_trifecta(win_c, win_b, win_a)
}
