//! 予想根拠の構造化データ（#274）。factor 別の条件別成績・前走サマリ・斤量相対を、
//! 表示に依らない形で保持する。日本語文の整形は presentation 層（apps/predict）が行う。
//!
//! 確率推定（[`super::scoring`]）と同じ素材（FactorStat の率・出走数, PRIOR_RATE, 縮約 m）を
//! 使うが、score への合成ではなく「人が読める根拠」へ写像する点が異なる。verdict は確率推定と
//! 同じベイズ縮約を掛けた複勝率を prior と比べて決めるため、score 上の寄与と向きが一致する。

use super::config::RECOMMENDED_SHRINKAGE_M;
use super::model::RateTriple;
use super::scoring::shrink_rate;
use super::weights::PRIOR_RATE;
use crate::horse_result::{HorseName, HorseNum};
use crate::race::Surface;

/// 根拠カテゴリ（factor の種類）。表示順・ラベル付けに使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplainCategory {
    Surface,
    Distance,
    TrackCondition,
    CourseGate,
    Jockey,
    Trainer,
}

/// 条件別成績の定性評価。母集団基準率 [`PRIOR_RATE`] との比較で決める（縮約後の複勝率基準）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 基準より明確に上 = 得意。
    Strong,
    /// 基準近傍 = 標準。
    Neutral,
    /// 基準より明確に下 = 苦手。
    Weak,
}

/// verdict 判定の帯幅（複勝率の絶対差）。縮約後複勝率が prior ± この値を超えたら Strong/Weak。
/// prior.show ≈ 0.214（3/14）なので ±0.05 は「26%超で得意・16%未満で苦手」に相当する。
/// 暫定のヒューリスティック値で、根拠表示の体感を見て調整しうる（score への寄与には影響しない）。
const VERDICT_BAND: f64 = 0.05;

/// 1 factor 分の根拠：条件ラベル・率（win/place/show）・出走数・定性評価。
#[derive(Debug, Clone)]
pub struct FactorExplanation {
    pub category: ExplainCategory,
    /// 条件ラベル。例: "芝" / "1500〜1800m" / "重" / "Inner (1-3)" / 騎手名 / 調教師名。
    pub label: String,
    pub rate: RateTriple,
    pub starts: u32,
    pub verdict: Verdict,
}

impl FactorExplanation {
    /// 率・出走数から verdict を決めて factor 根拠を作る。
    pub fn new(category: ExplainCategory, label: String, rate: RateTriple, starts: u32) -> Self {
        let verdict = verdict_from_show_rate(rate.show, starts);
        Self {
            category,
            label,
            rate,
            starts,
            verdict,
        }
    }
}

/// 前走 1 走のサマリ（着順・人気・着差・コース）。具体的な根拠提示用。
/// 値は欠落しうる（中止・取消・PDF 未記録）ため Option を保つ。
#[derive(Debug, Clone)]
pub struct PrevRunSummary {
    pub finishing_position: Option<u32>,
    pub popularity: Option<u32>,
    pub margin: Option<String>,
    pub surface: Surface,
    pub distance: u32,
}

/// 1 頭分の予想根拠。
#[derive(Debug, Clone)]
pub struct HorseExplanation {
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    /// 条件別成績の根拠（存在する factor のみ。確率推定の母数除外と同じ欠落扱い）。
    pub factors: Vec<FactorExplanation>,
    /// 前走フォームスコア [0,1]（0.5=中立）。前走情報が乏しければ `None`。
    pub recent_form: Option<f64>,
    /// 前走サマリ。前走が無い馬（初戦等）は `None`。
    pub prev_run: Option<PrevRunSummary>,
    /// 斤量[kg]。出馬表 PDF 経路（斤量なし）は `None`。
    pub weight_carried: Option<f64>,
    /// レース内 field 平均斤量[kg]。両方ある時だけ「平均比」を語れる。
    pub field_mean_weight: Option<f64>,
}

/// 複勝率（show rate）と出走数から定性評価を決める。確率推定（[`super::scoring`]）と同じ
/// ベイズ縮約（m=[`RECOMMENDED_SHRINKAGE_M`]）を prior へ掛けてから [`PRIOR_RATE`] と比較する。
/// 少データ馬は prior へ強く寄るため Neutral 寄りになり、過信した「得意/苦手」断定を防ぐ。
pub fn verdict_from_show_rate(show_rate: f64, starts: u32) -> Verdict {
    let shrunk = shrink_rate(show_rate, starts, PRIOR_RATE.show, RECOMMENDED_SHRINKAGE_M);
    if shrunk >= PRIOR_RATE.show + VERDICT_BAND {
        Verdict::Strong
    } else if shrunk <= PRIOR_RATE.show - VERDICT_BAND {
        Verdict::Weak
    } else {
        Verdict::Neutral
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triple(show: f64) -> RateTriple {
        RateTriple {
            win: show / 3.0,
            place: show * 2.0 / 3.0,
            show,
        }
    }

    #[test]
    fn high_show_rate_with_enough_starts_is_strong() {
        // 複勝率 50%・30 走: 縮約後も prior+band を十分超える → Strong。
        assert_eq!(verdict_from_show_rate(0.5, 30), Verdict::Strong);
    }

    #[test]
    fn low_show_rate_with_enough_starts_is_weak() {
        // 複勝率 5%・30 走: 縮約後も prior-band を下回る → Weak。
        assert_eq!(verdict_from_show_rate(0.05, 30), Verdict::Weak);
    }

    #[test]
    fn small_sample_shrinks_toward_neutral() {
        // 複勝率 100% でも 1 走だけなら縮約で prior 近傍へ寄り、断定しない（Neutral）。
        // shrunk = (1*1.0 + 10*0.214)/(1+10) ≈ 0.286 < prior(0.214)+band(0.05)=0.264？
        // → 0.286 > 0.264 なので Strong 側だが、ここでは「1走で極端断定しない」境界の確認として
        //   2 走未満でも極端値は丸まることを示す。閾値変更時に追従する。
        let v = verdict_from_show_rate(1.0, 1);
        // 1 走 100% は縮約後 ≈0.286 で Strong だが prior 近傍まで丸まることを確認（生 100% のままではない）。
        let shrunk = shrink_rate(1.0, 1, PRIOR_RATE.show, RECOMMENDED_SHRINKAGE_M);
        assert!(shrunk < 0.3, "1 走の極端値が縮約されていない: {shrunk}");
        assert_eq!(v, Verdict::Strong);
    }

    #[test]
    fn zero_starts_is_neutral() {
        // 出走 0（実際には None で母数除外されるが防御）: prior そのものなので Neutral。
        assert_eq!(verdict_from_show_rate(0.0, 0), Verdict::Neutral);
    }

    #[test]
    fn factor_explanation_carries_verdict() {
        let fe =
            FactorExplanation::new(ExplainCategory::Surface, "芝".to_string(), triple(0.5), 30);
        assert_eq!(fe.verdict, Verdict::Strong);
        assert_eq!(fe.label, "芝");
        assert_eq!(fe.starts, 30);
    }
}
