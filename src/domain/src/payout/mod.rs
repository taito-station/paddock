//! 確定払戻（配当）と予想セッションの自動精算（純粋ロジック・IO なし）。
//!
//! netkeiba の確定払戻を [`RacePayouts`] に保持し、`predict_bets`(bet_type, combination) と
//! **文字列一致**で照合して払戻額を算出する（[`settle_bet`]）。券種ラベルは
//! [`crate::BetCombination::type_label`] と、組合せコードは
//! [`crate::BetCombination::combination_code`] と同形式に揃える前提。
//!
//! 出走取消(取)・競走除外(除)の馬を含む組番は JRA ルールで**全額返還**されるため、
//! `RacePayouts` は返還対象馬番（`scratched`）も保持し、[`settle_bet`] は返還を stake 返戻として扱う。

use std::collections::{HashMap, HashSet};

use crate::race::RaceId;

/// 1 レース分の確定払戻。`(券種ラベル, 組合せコード) -> 100 円あたり配当(円)`。
///
/// 複勝・ワイドは的中組合せが複数、同着は同一券種に複数の的中組合せが並ぶが、いずれも
/// 別キーとして保持されるため自然に表現できる。払戻が 1 件も無いレースは未確定とみなす。
///
/// `scratched` は出走取消・競走除外の馬番集合。これらを 1 頭でも含む組番は返還対象になる。
#[derive(Debug, Clone)]
pub struct RacePayouts {
    pub race_id: RaceId,
    entries: HashMap<(String, String), u32>,
    scratched: HashSet<u32>,
}

impl RacePayouts {
    /// 空の払戻集合を作る（呼び出し側が券種ごとに `insert` する）。
    pub fn empty(race_id: RaceId) -> Self {
        Self {
            race_id,
            entries: HashMap::new(),
            scratched: HashSet::new(),
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

    /// 返還対象馬番（出走取消・競走除外）の集合を設定する。
    pub fn set_scratched(&mut self, scratched: HashSet<u32>) {
        self.scratched = scratched;
    }

    /// `combo_code` 内の馬番に返還対象（取消/除外）が 1 頭でも含まれるか。
    ///
    /// JRA では組番に非出走馬を含むと当該組番は**全額返還**（按分なし・組番単位 all-or-nothing）。
    /// 各 `predict_bets` 行＝1 組番なので、流しの一部のみ取消でも該当行だけが返還になる。
    pub fn is_refunded(&self, combo_code: &str) -> bool {
        !self.scratched.is_empty() && combo_nums(combo_code).any(|n| self.scratched.contains(&n))
    }
}

/// 組合せコード（例 `8` / `6-8` / `1>2>3`）を構成馬番に分解する。区切りは `-`（無順）と `>`（順序付き）。
fn combo_nums(combo_code: &str) -> impl Iterator<Item = u32> + '_ {
    combo_code
        .split(['-', '>'])
        .filter_map(|s| s.parse::<u32>().ok())
}

/// 1 買い目の精算結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settlement {
    /// 的中。配当から算出した払戻額(円)。
    Hit(u64),
    /// 返還（組番に取消/除外馬を含む）。`stake` を全額返戻する。
    Refund(u64),
    /// 不的中（払戻 0）。
    Miss,
}

impl Settlement {
    /// 払戻額(円)。返還は stake、不的中は 0。
    pub fn payout(self) -> u64 {
        match self {
            Settlement::Hit(p) | Settlement::Refund(p) => p,
            Settlement::Miss => 0,
        }
    }

    /// 返還（取消/除外馬を含む組番）か。
    pub fn is_refund(self) -> bool {
        matches!(self, Settlement::Refund(_))
    }
}

/// 確定払戻から 1 買い目を精算する純関数。
///
/// 判定は次の優先順で行う:
/// 1. 組番に取消/除外馬を含むなら [`Settlement::Refund`]（`stake` 全額返戻）。**配当照合に優先する**。
/// 2. 配当を引けたら [`Settlement::Hit`]（`stake / 100 * payoff_per_100`、JRA の 100 円あたり払戻）。
/// 3. いずれでもなければ [`Settlement::Miss`]。
///
/// `stake` は 100 円単位前提（端数は切り捨て）。返還の評価は呼び出し側で重複しないよう、
/// 払戻額と返還判定を [`Settlement`] にまとめて返す。
pub fn settle_bet(
    type_label: &str,
    combo_code: &str,
    stake: u64,
    payouts: &RacePayouts,
) -> Settlement {
    if payouts.is_refunded(combo_code) {
        return Settlement::Refund(stake);
    }
    match payouts.payoff(type_label, combo_code) {
        Some(per100) => Settlement::Hit(stake / 100 * u64::from(per100)),
        None => Settlement::Miss,
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
        assert_eq!(
            settle_bet("quinella", "6-8", 600, &p),
            Settlement::Hit(7560)
        );
        // 単勝 8 を ¥5,000 → 5000/100 * 140 = ¥7,000
        assert_eq!(settle_bet("win", "8", 5000, &p), Settlement::Hit(7000));
    }

    #[test]
    fn miss_pays_zero() {
        let p = payouts();
        assert_eq!(settle_bet("quinella", "3-8", 600, &p), Settlement::Miss);
        assert_eq!(settle_bet("trifecta", "1>2>3", 100, &p), Settlement::Miss);
        assert_eq!(settle_bet("quinella", "3-8", 600, &p).payout(), 0);
    }

    #[test]
    fn dead_heat_keeps_both_winning_combos() {
        let p = payouts();
        assert_eq!(settle_bet("quinella", "6-8", 100, &p).payout(), 1260);
        assert_eq!(settle_bet("quinella", "1-2", 100, &p).payout(), 500);
    }

    #[test]
    fn empty_is_unconfirmed() {
        let p = RacePayouts::empty(RaceId::try_from("2026-3-tokyo-3-1R").unwrap());
        assert!(p.is_empty());
        assert_eq!(settle_bet("win", "8", 100, &p), Settlement::Miss);
    }

    #[test]
    fn stake_not_multiple_of_100_floors() {
        let p = payouts();
        // ¥150 → 1 単位ぶんのみ（端数切り捨て）: 150/100=1 * 140 = 140
        assert_eq!(settle_bet("win", "8", 150, &p), Settlement::Hit(140));
    }

    /// scratched を持つ払戻集合（馬番 6 が取消/除外）。
    fn payouts_with_scratch() -> RacePayouts {
        let mut p = payouts();
        p.set_scratched(HashSet::from([6]));
        p
    }

    #[test]
    fn scratched_horse_in_single_bet_is_refunded() {
        let p = payouts_with_scratch();
        // 単勝 6 は取消 → 配当表に無くても stake 全額返戻。
        assert_eq!(settle_bet("win", "6", 5000, &p), Settlement::Refund(5000));
        // 複勝 6 も同様に返還。
        assert_eq!(settle_bet("place", "6", 600, &p), Settlement::Refund(600));
    }

    #[test]
    fn combo_containing_scratched_horse_is_refunded() {
        let p = payouts_with_scratch();
        // 馬連 6-8 は本来 ¥1,260 的中だが、6 が取消なので組番ごと全額返還（配当照合に優先）。
        assert_eq!(
            settle_bet("quinella", "6-8", 600, &p),
            Settlement::Refund(600)
        );
        // 三連複 4-6-8 も 6 を含むため返還（配当表に無くても stake）。
        assert_eq!(
            settle_bet("trio", "4-6-8", 300, &p),
            Settlement::Refund(300)
        );
    }

    #[test]
    fn nagashi_refunds_only_rows_with_scratched_horse() {
        // 流し（複数組番）のうち取消馬を含む行だけ返還・他行は通常精算される。
        let p = payouts_with_scratch();
        // 6 を含む 6-8 は返還（stake）。
        assert_eq!(
            settle_bet("quinella", "6-8", 600, &p),
            Settlement::Refund(600)
        );
        // 6 を含まない 1-2 は同着的中で通常払戻。
        assert_eq!(
            settle_bet("quinella", "1-2", 600, &p),
            Settlement::Hit(3000)
        );
        // 6 を含まず不的中の 7-8 は 0 のまま。
        assert_eq!(settle_bet("quinella", "7-8", 600, &p), Settlement::Miss);
    }

    #[test]
    fn no_scratch_keeps_existing_behavior() {
        // 取消が無いレースは従来どおり（is_refunded は常に false）。
        let p = payouts();
        assert!(!p.is_refunded("6-8"));
        assert_eq!(
            settle_bet("quinella", "6-8", 600, &p),
            Settlement::Hit(7560)
        );
        assert_eq!(settle_bet("quinella", "3-8", 600, &p), Settlement::Miss);
    }

    #[test]
    fn settlement_payout_and_is_refund() {
        assert_eq!(Settlement::Hit(7560).payout(), 7560);
        assert_eq!(Settlement::Refund(600).payout(), 600);
        assert_eq!(Settlement::Miss.payout(), 0);
        assert!(Settlement::Refund(600).is_refund());
        assert!(!Settlement::Hit(7560).is_refund());
        assert!(!Settlement::Miss.is_refund());
    }

    #[test]
    fn is_refunded_detects_horse_in_any_position() {
        let p = payouts_with_scratch();
        assert!(p.is_refunded("6")); // 単一
        assert!(p.is_refunded("6-8")); // 無順の先頭
        assert!(p.is_refunded("8-6")); // 無順の末尾
        assert!(p.is_refunded("1>6>3")); // 順序付きの中間
        assert!(!p.is_refunded("7-8")); // 取消馬を含まない
        // 「16」を「1」「6」と取り違えない（区切りで分解するため）。
        assert!(!p.is_refunded("16-8"));
    }
}
