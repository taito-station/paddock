//! 確定着順に対する買い目の的中判定（backtest 校正用, #121）。

use super::model::{BetCombination, Podium};
use crate::horse_result::HorseNum;

/// 買い目 `combination` が確定着順 `podium` で的中したか（#121, backtest 校正用）。
///
/// 着順が揃わない券種（例: 1〜2 着が未確定での馬連）は `false`（非的中扱い）。
/// - 単勝: 1 着一致 / 複勝: 払戻圏（8 頭以上＝3 着以内・7 頭以下＝2 着以内）/ 馬連: {1,2着}＝無順ペア / 馬単: 1→2 着完全一致
/// - ワイド: 両馬が払戻圏 / 三連複: {1,2,3着}＝無順トリプル / 三連単: 1→2→3 着完全一致
pub fn bet_hit(combination: &BetCombination, podium: &Podium) -> bool {
    match combination {
        BetCombination::Win(h) => podium.first == Some(*h),
        BetCombination::Place(h) => podium.in_the_money(*h),
        BetCombination::Quinella(p) => {
            let (a, b) = p.as_tuple();
            unordered_pair_eq(podium.first, podium.second, a, b)
        }
        BetCombination::Wide(p) => {
            let (a, b) = p.as_tuple();
            podium.in_the_money(a) && podium.in_the_money(b)
        }
        BetCombination::Exacta(p) => {
            let (a, b) = p.as_tuple();
            podium.first == Some(a) && podium.second == Some(b)
        }
        BetCombination::Trio(t) => {
            let (a, b, c) = t.as_tuple();
            unordered_triple_eq(podium, a, b, c)
        }
        BetCombination::Trifecta(t) => {
            let (a, b, c) = t.as_tuple();
            podium.first == Some(a) && podium.second == Some(b) && podium.third == Some(c)
        }
    }
}

/// {first, second}（ともに Some）が無順で {a, b} に一致するか。
fn unordered_pair_eq(
    first: Option<HorseNum>,
    second: Option<HorseNum>,
    a: HorseNum,
    b: HorseNum,
) -> bool {
    match (first, second) {
        (Some(f), Some(s)) => (f == a && s == b) || (f == b && s == a),
        _ => false,
    }
}

/// {1,2,3着}（すべて Some）が無順で {a, b, c} に一致するか。
fn unordered_triple_eq(podium: &Podium, a: HorseNum, b: HorseNum, c: HorseNum) -> bool {
    match (podium.first, podium.second, podium.third) {
        (Some(f), Some(s), Some(t)) => {
            let mut got = [f.value(), s.value(), t.value()];
            let mut want = [a.value(), b.value(), c.value()];
            got.sort_unstable();
            want.sort_unstable();
            got == want
        }
        _ => false,
    }
}
