//! 確定払戻（配当）と予想セッションの自動精算（純粋ロジック・IO なし）。
//!
//! netkeiba の確定払戻を [`RacePayouts`] に保持し、`predict_bets`(bet_type, combination) と
//! **文字列一致**で照合して払戻額を算出する（[`settle_bet`]）。券種ラベルは
//! [`crate::BetCombination::type_label`] と、組合せコードは
//! [`crate::BetCombination::combination_code`] と同形式に揃える前提。

use std::collections::HashMap;

use crate::race::RaceId;

/// 1 レース分の確定払戻。`(券種ラベル, 組合せコード) -> 100 円あたり配当(円)`。
///
/// 複勝・ワイドは的中組合せが複数、同着は同一券種に複数の的中組合せが並ぶが、いずれも
/// 別キーとして保持されるため自然に表現できる。払戻が 1 件も無いレースは未確定とみなす。
#[derive(Debug, Clone)]
pub struct RacePayouts {
    pub race_id: RaceId,
    entries: HashMap<(String, String), u32>,
}

impl RacePayouts {
    /// 空の払戻集合を作る（呼び出し側が券種ごとに `insert` する）。
    pub fn empty(race_id: RaceId) -> Self {
        Self {
            race_id,
            entries: HashMap::new(),
        }
    }

    /// 的中組合せの 100 円あたり配当を登録する。
    /// `type_label` は `type_label()`、`combo_code` は `combination_code()` と同形式で渡す。
    pub fn insert(
        &mut self,
        type_label: impl Into<String>,
        combo_code: impl Into<String>,
        payoff_per_100: u32,
    ) {
        self.entries
            .insert((type_label.into(), combo_code.into()), payoff_per_100);
    }

    /// 指定券種・組合せの 100 円あたり配当を引く。不的中（未登録）なら `None`。
    pub fn payoff(&self, type_label: &str, combo_code: &str) -> Option<u32> {
        self.entries
            .get(&(type_label.to_string(), combo_code.to_string()))
            .copied()
    }

    /// 払戻が 1 件も無い（＝未確定）か。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 登録済みの的中組合せ数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// 確定払戻から 1 買い目の払戻額(円)を算出する純関数。
///
/// 配当を引けたら `stake / 100 * payoff_per_100`（JRA の 100 円あたり払戻）、引けなければ 0。
/// `stake` は 100 円単位前提（端数は切り捨て）。
pub fn settle_bet(type_label: &str, combo_code: &str, stake: u64, payouts: &RacePayouts) -> u64 {
    match payouts.payoff(type_label, combo_code) {
        Some(per100) => stake / 100 * u64::from(per100),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payouts() -> RacePayouts {
        let mut p = RacePayouts::empty(RaceId::try_from("2026-3-tokyo-3-1R").unwrap());
        p.insert("win", "8", 140);
        p.insert("quinella", "6-8", 1260); // 馬連 6-8 ¥1,260
        // 同着の馬連（同一券種に 2 組）
        p.insert("quinella", "1-2", 500);
        // 複勝・ワイドは的中組合せが複数
        p.insert("place", "8", 110);
        p.insert("place", "6", 150);
        p.insert("wide", "6-8", 420);
        p
    }

    #[test]
    fn settles_hit_per_100yen() {
        let p = payouts();
        // 馬連 6-8 を ¥600 → 600/100 * 1260 = ¥7,560
        assert_eq!(settle_bet("quinella", "6-8", 600, &p), 7560);
        // 単勝 8 を ¥5,000 → 5000/100 * 140 = ¥7,000
        assert_eq!(settle_bet("win", "8", 5000, &p), 7000);
    }

    #[test]
    fn miss_pays_zero() {
        let p = payouts();
        assert_eq!(settle_bet("quinella", "3-8", 600, &p), 0);
        assert_eq!(settle_bet("trifecta", "1>2>3", 100, &p), 0);
    }

    #[test]
    fn dead_heat_keeps_both_winning_combos() {
        let p = payouts();
        assert_eq!(settle_bet("quinella", "6-8", 100, &p), 1260);
        assert_eq!(settle_bet("quinella", "1-2", 100, &p), 500);
    }

    #[test]
    fn empty_is_unconfirmed() {
        let p = RacePayouts::empty(RaceId::try_from("2026-3-tokyo-3-1R").unwrap());
        assert!(p.is_empty());
        assert_eq!(settle_bet("win", "8", 100, &p), 0);
    }

    #[test]
    fn stake_not_multiple_of_100_floors() {
        let p = payouts();
        // ¥150 → 1 単位ぶんのみ（端数切り捨て）: 150/100=1 * 140 = 140
        assert_eq!(settle_bet("win", "8", 150, &p), 140);
    }
}
