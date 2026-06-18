//! 投票推奨のドメイン値オブジェクト（設定・買い目・推奨・確定着順）。

use crate::horse_result::HorseNum;
use crate::odds::{BetType, OrderedPair, OrderedTriple, Pair, Triple};

#[derive(Debug, Clone)]
pub struct BettingConfig {
    pub ev_threshold: f64,
    pub trifecta_ev_threshold: f64,
    pub kelly_cap: f64,
    /// curation（#121）: Kelly 分数がこの値以下の買い目を除外する（`retain(kelly > min_kelly)`）。
    /// EV がわずかに正でも Kelly≈0 の薄い買い目（全正EVダンプの主因）を落とす主要レバー。
    /// `0.0` で無効＝従来挙動: EV 閾値通過（ev>1.0）の買い目は必ず Kelly>0（`f=(ev-1)/b`）なので、
    /// `min_kelly=0.0` の strict `>` でも EV 通過分は 1 点も落とさない。
    pub min_kelly: f64,
    /// curation（#121）: 券種ごとに EV 上位 N 点に制限する。`None` で無制限＝従来挙動。
    /// 全組合せ羅列（1R 数千点）を実用的な点数に抑える。
    pub max_bets_per_type: Option<usize>,
}

impl Default for BettingConfig {
    /// 本番 predict が使う既定値。EV 閾値に加え curation（min_kelly / 券種別上限）で
    /// 全正EVダンプ（#121）を抑える。curation 値は backtest（exotic 校正・回収率）で裏付ける。
    fn default() -> Self {
        Self {
            ev_threshold: 1.0,
            trifecta_ev_threshold: 2.0,
            kelly_cap: 0.25,
            min_kelly: 0.01,
            max_bets_per_type: Some(8),
        }
    }
}

#[derive(Debug, Clone)]
pub enum BetCombination {
    Win(HorseNum),
    Place(HorseNum),
    Quinella(Pair),
    Wide(Pair),
    Exacta(OrderedPair),
    Trio(Triple),
    Trifecta(OrderedTriple),
}

impl BetCombination {
    /// この買い目の券種。
    pub fn bet_type(&self) -> BetType {
        match self {
            BetCombination::Win(_) => BetType::Win,
            BetCombination::Place(_) => BetType::Place,
            BetCombination::Quinella(_) => BetType::Quinella,
            BetCombination::Wide(_) => BetType::Wide,
            BetCombination::Exacta(_) => BetType::Exacta,
            BetCombination::Trio(_) => BetType::Trio,
            BetCombination::Trifecta(_) => BetType::Trifecta,
        }
    }

    /// 馬券種を表す安定した小文字ラベル（DB 保存・分析用）。
    pub fn type_label(&self) -> &'static str {
        match self {
            BetCombination::Win(_) => "win",
            BetCombination::Place(_) => "place",
            BetCombination::Quinella(_) => "quinella",
            BetCombination::Wide(_) => "wide",
            BetCombination::Exacta(_) => "exacta",
            BetCombination::Trio(_) => "trio",
            BetCombination::Trifecta(_) => "trifecta",
        }
    }

    /// 組み合わせを表す文字列コード（DB 保存・表示用）。
    /// 無順（馬連/三連複）は `-` 区切りで昇順、順序付き（馬単/三連単）は `>` 区切り。
    /// 例: 単勝 `"3"` / 馬連 `"1-5"` / 馬単 `"1>5"` / 三連複 `"1-3-5"` / 三連単 `"1>3>5"`。
    pub fn combination_code(&self) -> String {
        match self {
            BetCombination::Win(h) | BetCombination::Place(h) => h.value().to_string(),
            BetCombination::Quinella(p) | BetCombination::Wide(p) => {
                let (a, b) = p.as_tuple();
                format!("{}-{}", a.value(), b.value())
            }
            BetCombination::Exacta(p) => {
                let (a, b) = p.as_tuple();
                format!("{}>{}", a.value(), b.value())
            }
            BetCombination::Trio(t) => {
                let (a, b, c) = t.as_tuple();
                format!("{}-{}-{}", a.value(), b.value(), c.value())
            }
            BetCombination::Trifecta(t) => {
                let (a, b, c) = t.as_tuple();
                format!("{}>{}>{}", a.value(), b.value(), c.value())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct BettingRecommendation {
    pub combination: BetCombination,
    pub probability: f64,
    /// Gross payout multiplier. `ev = probability * odds`.
    /// - 単勝/馬連/馬単/三連複/三連単: JRA が公表するオッズそのまま
    /// - 複勝（`BetCombination::Place`）: オッズ幅の算術平均 `(low + high) / 2.0` を代入
    pub odds: f64,
    pub ev: f64,
    pub kelly_fraction: f64,
}

/// 確定レースの上位 3 着（着順 → 馬番）と出走頭数。同着や着順欠落は `None`。backtest の買い目的中判定用（#121）。
#[derive(Debug, Clone, Copy, Default)]
pub struct Podium {
    pub first: Option<HorseNum>,
    pub second: Option<HorseNum>,
    pub third: Option<HorseNum>,
    /// 出走頭数。複勝/ワイドの払戻圏が頭数依存（JRA: 8 頭以上＝3 着以内、7 頭以下＝2 着以内）の
    /// ため判定に使う。0（既定）は払戻圏を 2 着以内に倒す保守側になる。
    pub field_size: usize,
}

impl Podium {
    /// 馬番が複勝/ワイドの払戻圏に居るか。JRA の複勝は出走 8 頭以上で 3 着以内、
    /// 7 頭以下（5〜7 頭）では 2 着以内のみが払戻対象で 3 着は不的中（4 頭以下は複勝非発売）。
    /// この頭数依存を反映しないと小頭数レースで複勝/ワイドの的中・回収率が楽観側へ歪む。
    pub(crate) fn in_the_money(&self, h: HorseNum) -> bool {
        let target = Some(h);
        if self.first == target || self.second == target {
            return true;
        }
        // 3 着が払戻圏に入るのは 8 頭以上のときだけ。
        self.field_size >= 8 && self.third == target
    }
}
