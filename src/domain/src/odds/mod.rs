mod bet_type;
mod combination;
mod odds_value;

pub use bet_type::BetType;
pub use combination::{OrderedPair, OrderedTriple, Pair, Triple};
pub use odds_value::{OddsValue, PlaceOdds};

use std::collections::HashMap;

use crate::horse_result::HorseNum;
use crate::race::RaceId;

/// All bet-type odds maps scraped for a single race.
///
/// Each map is keyed by the bet combination and holds the quoted odds. Maps are
/// independent: a pool that JRA has not published yet is simply left empty.
#[derive(Debug, Clone)]
pub struct RaceOdds {
    pub race_id: RaceId,
    /// 単勝
    pub win: HashMap<HorseNum, OddsValue>,
    /// 複勝 (low..high band per horse)
    pub place: HashMap<HorseNum, PlaceOdds>,
    /// 馬連
    pub quinella: HashMap<Pair, OddsValue>,
    /// ワイド (low..high band per pair)。オッズスクレイパが populate する想定
    /// (#25)。収支シミュレータは買い目ごとの確定オッズを使うため本フィールドは参照しない。
    pub wide: HashMap<Pair, PlaceOdds>,
    /// 馬単
    pub exacta: HashMap<OrderedPair, OddsValue>,
    /// 三連複
    pub trio: HashMap<Triple, OddsValue>,
    /// 三連単
    pub trifecta: HashMap<OrderedTriple, OddsValue>,
}

impl RaceOdds {
    /// Create an empty odds set for a race; callers fill the per-bet-type maps.
    pub fn empty(race_id: RaceId) -> Self {
        Self {
            race_id,
            win: HashMap::new(),
            place: HashMap::new(),
            quinella: HashMap::new(),
            wide: HashMap::new(),
            exacta: HashMap::new(),
            trio: HashMap::new(),
            trifecta: HashMap::new(),
        }
    }

    /// True when no bet type has any quoted odds.
    pub fn is_empty(&self) -> bool {
        self.win.is_empty()
            && self.place.is_empty()
            && self.quinella.is_empty()
            && self.wide.is_empty()
            && self.exacta.is_empty()
            && self.trio.is_empty()
            && self.trifecta.is_empty()
    }

    /// read-through cache-hit 用の「完全な（再スクレイプ不要な）スナップショット」判定（#294）。
    /// win と全組合せ券種（馬連・ワイド・馬単・三連複・三連単）が揃っていれば true。
    ///
    /// `place` は除外する: netkeiba は win と同梱で通常そろうが、ADR 0010 の「複勝未公開時も
    /// win-only で cache-hit を許容」を維持し、発走前の place 未公開で再スクレイプが無限化するのを
    /// 避けるため。netkeiba は win と組合せ券種をほぼ同時公開するので、「win あり・組合せ欠落」は
    /// 一過性の取得失敗に限られ、これを cache-miss として取り直すのが本判定の狙い。
    ///
    /// 組合せ 5 券種すべてを要求するのは「健全なスクレイプが返すフルの形」を完全性の基準にするため
    /// （買い目に使わない馬単・三連単も api-server 配信や将来用途のため欠落を検知して取り直す）。
    /// 副作用として、JRA が一部の組合せ券種を発売しない極小頭数レースでは常に false になり
    /// read-through で毎回再スクレイプするが、`race_odds` は UPSERT で行が肥大せず呼び出しも
    /// 1 レース 1 回程度のため許容する（#294 影響: 低。詳細は OddsInteractor::race_odds のコメント）。
    pub fn is_complete(&self) -> bool {
        !self.win.is_empty()
            && !self.quinella.is_empty()
            && !self.wide.is_empty()
            && !self.exacta.is_empty()
            && !self.trio.is_empty()
            && !self.trifecta.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }
    fn ov(v: f64) -> OddsValue {
        OddsValue::try_from(v).unwrap()
    }
    fn rid() -> RaceId {
        RaceId::try_from("2026-3-nakayama-8-1R").unwrap()
    }

    /// win + 全組合せ 5 券種を入れた complete なスナップショットを作る（place はあえて入れない）。
    fn complete_odds() -> RaceOdds {
        let mut o = RaceOdds::empty(rid());
        o.win.insert(h(1), ov(3.5));
        o.quinella
            .insert(Pair::try_from((h(1), h(2))).unwrap(), ov(12.4));
        o.wide.insert(
            Pair::try_from((h(1), h(2))).unwrap(),
            PlaceOdds::try_from((ov(3.1), ov(4.8))).unwrap(),
        );
        o.exacta
            .insert(OrderedPair::try_from((h(2), h(1))).unwrap(), ov(25.0));
        o.trio
            .insert(Triple::try_from((h(1), h(2), h(3))).unwrap(), ov(88.0));
        o.trifecta.insert(
            OrderedTriple::try_from((h(3), h(1), h(2))).unwrap(),
            ov(410.0),
        );
        o
    }

    #[test]
    fn is_complete_true_when_win_and_all_exotic_present() {
        // place 無しでも win + 5 券種そろえば complete（place は判定対象外）。
        assert!(complete_odds().is_complete());
    }

    #[test]
    fn is_complete_false_when_an_exotic_type_missing() {
        // 三連複だけ欠けた部分スナップショット → cache-miss（再スクレイプ対象）。
        let mut o = complete_odds();
        o.trio.clear();
        assert!(!o.is_complete());
    }

    #[test]
    fn is_complete_false_for_win_only() {
        // win のみ（組合せ全欠落）は complete でない。
        let mut o = RaceOdds::empty(rid());
        o.win.insert(h(1), ov(3.5));
        assert!(!o.is_complete());
    }

    #[test]
    fn is_complete_false_for_empty() {
        assert!(!RaceOdds::empty(rid()).is_complete());
    }
}
