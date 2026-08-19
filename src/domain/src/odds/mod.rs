mod bet_type;
mod combination;
mod odds_value;

pub use bet_type::BetType;
pub use combination::{OrderedPair, OrderedTriple, Pair, Triple};
pub use odds_value::{OddsValue, PlaceOdds};

use std::collections::{BTreeSet, HashMap};

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
    ///
    /// 本判定は**券種が実際に発売されているかを知らない**。JRA が売らない券種や発売開始前の
    /// 時間帯では永久に false になるため、read-through の cache-hit をこれ単体で決めてはいけない
    /// （毎回再スクレイプになる・#632）。「欠けているが未発売と確認済み」を差し引く責務は
    /// use-case 層（`OddsInteractor::race_odds`）が持つ。本メソッドの意味は
    /// 「priced な行が全券種そろっているか」のまま据え置き、`find_race_odds_morning` の
    /// 「朝時点＝最初にフル盤が成立した snapshot」判定はこの意味に依存している（ADR 0088）。
    pub fn is_complete(&self) -> bool {
        self.missing_bet_types().is_empty()
    }

    /// `is_complete()` が要求する券種のうち、priced な行を 1 つも持たないものを返す（#632）。
    ///
    /// read-through の cache-hit 判定で「欠けている券種が未発売と確認済みの集合に収まるか」を
    /// 突き合わせるために使う。`is_complete()` と同じ券種集合を見る（`place` は含めない）ので、
    /// 両者の判定基準がズレない（second source を作らない）。
    pub fn missing_bet_types(&self) -> BTreeSet<BetType> {
        let mut missing = BTreeSet::new();
        if self.win.is_empty() {
            missing.insert(BetType::Win);
        }
        if self.quinella.is_empty() {
            missing.insert(BetType::Quinella);
        }
        if self.wide.is_empty() {
            missing.insert(BetType::Wide);
        }
        if self.exacta.is_empty() {
            missing.insert(BetType::Exacta);
        }
        if self.trio.is_empty() {
            missing.insert(BetType::Trio);
        }
        if self.trifecta.is_empty() {
            missing.insert(BetType::Trifecta);
        }
        missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }
    fn ov(v: f64) -> OddsValue {
        // テスト値はどの券種の番兵とも重ならない正当オッズ。非番兵値のガード結果は券種に
        // 依らないため、ヘルパは Win 固定で包む（#630）。
        OddsValue::try_from((BetType::Win, v)).unwrap()
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

    #[test]
    fn missing_bet_types_never_includes_place() {
        // **use-case 側の安全性がこの不変条件に乗っている**（#632）。`is_cache_fresh` は
        // 未発売観測から `Win` だけを除外し `Place` は素通しにしているが、それが安全なのは
        // 「`missing` に `Place` が入らないので `fresh` 側に混ざっても is_subset を変えない」
        // から。ここが崩れると、複勝の観測が欠落を免除して cache-hit を通してしまう。
        // 複勝が空でも満杯でも `Place` は出ない、を両方向で固定する。
        let mut empty_place = complete_odds();
        empty_place.place.clear();
        assert!(!empty_place.missing_bet_types().contains(&BetType::Place));
        assert!(
            empty_place.is_complete(),
            "place は is_complete の判定対象外（ADR 0010）"
        );

        assert!(
            !RaceOdds::empty(rid())
                .missing_bet_types()
                .contains(&BetType::Place),
            "全券種が空でも Place は欠落として挙げない"
        );
    }

    #[test]
    fn missing_bet_types_lists_exactly_the_empty_cache_bet_types() {
        let mut o = complete_odds();
        o.trio.clear();
        o.trifecta.clear();
        assert_eq!(
            o.missing_bet_types(),
            BTreeSet::from([BetType::Trio, BetType::Trifecta])
        );

        // 空のオッズは win + 組合せ 5 券種＝6 券種すべてが欠落（place は含まない）。
        assert_eq!(
            RaceOdds::empty(rid()).missing_bet_types(),
            BTreeSet::from([
                BetType::Win,
                BetType::Quinella,
                BetType::Wide,
                BetType::Exacta,
                BetType::Trio,
                BetType::Trifecta,
            ])
        );
    }
}
