//! 買い目ポートフォリオの収支シミュレータ（純粋ロジック・IO なし）。
//!
//! 単一レースの全着順（上位 3 着の順列）を列挙し、与えられた買い目集合の的中・払戻を
//! 計算して、ベストケース／「当たっても赤字」着順の有無／的中通り数／本線収支／
//! （各馬の単勝確率が与えられた場合は）期待払戻・期待回収率・的中確率を集計する。

use std::collections::HashMap;

use crate::betting::{BetCombination, harville_trifecta};
use crate::error::{Error, Result};
use crate::horse_result::HorseNum;
use crate::prediction::HorseProbability;

/// 上位 3 着の馬番（1 着, 2 着, 3 着）。
pub type Finish = (HorseNum, HorseNum, HorseNum);

/// 1 つの確定した買い目（券種・組合せ・賭け金・オッズ）。
#[derive(Debug, Clone)]
pub struct PlacedBet {
    pub combination: BetCombination,
    /// 賭け金（円）。
    pub stake: u64,
    /// 払戻倍率（JRA 公表オッズそのもの。100 円→`odds * 100` 円が払戻）。
    pub odds: f64,
}

/// シミュレータ入力（単一レース）。
#[derive(Debug, Clone)]
pub struct SimInput {
    /// 出走馬の馬番（着順列挙の母集合）。3 頭以上・重複なし。
    pub field: Vec<HorseNum>,
    /// 評価する買い目集合。
    pub bets: Vec<PlacedBet>,
    /// 本線（着目したい着順）。指定時はその収支も併せて出す。
    pub main: Option<Finish>,
    /// 各馬の単勝確率。与えられた場合のみ EV 等を算出する。
    pub win_probs: Option<Vec<HorseProbability>>,
}

/// ある着順における収支。
#[derive(Debug, Clone)]
pub struct Outcome {
    pub finish: Finish,
    /// 払戻合計（円）。
    pub payout: u64,
    /// 収支 = payout - total_stake。
    pub pnl: i128,
}

/// 単勝確率が与えられたときの期待値系の指標。
#[derive(Debug, Clone)]
pub struct EvReport {
    /// 期待払戻（円）。
    pub ev: f64,
    /// 期待回収率 = ev / total_stake。
    pub roi: f64,
    /// 的中確率（payout > 0 となる着順の確率和）。
    pub hit_prob: f64,
}

/// シミュレーション結果。
#[derive(Debug, Clone)]
pub struct SimReport {
    pub total_stake: u64,
    /// 全着順の通り数（= n·(n-1)·(n-2)）。
    pub total_count: u64,
    /// 1 つ以上の買い目が的中する着順の通り数。
    pub hit_count: u64,
    /// 最大払戻となる着順（ベストケース）。
    pub best: Outcome,
    /// 的中する着順のうち最小払戻のもの。`pnl < 0` なら「当たっても赤字」。
    /// 的中する着順が 1 つも無いときは `None`。
    pub worst_hit: Option<Outcome>,
    /// 本線（入力で指定された着順）の収支。未指定なら `None`。
    pub main: Option<Outcome>,
    /// 期待値系の指標（`win_probs` 指定時のみ）。
    pub ev: Option<EvReport>,
}

/// 買い目 `c` が着順 (first, second, third) で的中するか。`runners` は出走頭数。
fn is_hit(c: &BetCombination, first: HorseNum, second: HorseNum, third: HorseNum, runners: usize) -> bool {
    let in_top2 = |h: HorseNum| h == first || h == second;
    let in_top3 = |h: HorseNum| h == first || h == second || h == third;
    match c {
        BetCombination::Win(h) => *h == first,
        // 複勝: 出走 7 頭以下は 2 着以内、8 頭以上は 3 着以内（JRA ルール）。
        BetCombination::Place(h) => {
            if runners <= 7 {
                in_top2(*h)
            } else {
                in_top3(*h)
            }
        }
        // 馬連: 2 頭が 1・2 着（順不同）。Pair は相異なるので両方 top2 ⇔ {1着,2着}。
        BetCombination::Quinella(p) => {
            let (a, b) = p.as_tuple();
            in_top2(a) && in_top2(b)
        }
        // ワイド: 2 頭がともに 3 着以内。
        BetCombination::Wide(p) => {
            let (a, b) = p.as_tuple();
            in_top3(a) && in_top3(b)
        }
        // 馬単: 1 着→2 着が順序どおり一致。
        BetCombination::Exacta(p) => {
            let (a, b) = p.as_tuple();
            a == first && b == second
        }
        // 三連複: 3 頭が 1〜3 着（順不同）。
        BetCombination::Trio(t) => {
            let (a, b, c) = t.as_tuple();
            in_top3(a) && in_top3(b) && in_top3(c)
        }
        // 三連単: 1→2→3 着が順序どおり一致。
        BetCombination::Trifecta(t) => {
            let (a, b, c) = t.as_tuple();
            a == first && b == second && c == third
        }
    }
}

/// 着順 (first, second, third) における全買い目の払戻合計（円、端数は円未満切り捨て）。
fn payout_of(bets: &[PlacedBet], first: HorseNum, second: HorseNum, third: HorseNum, runners: usize) -> u64 {
    bets.iter()
        .filter(|b| is_hit(&b.combination, first, second, third, runners))
        .map(|b| (b.stake as f64 * b.odds).floor() as u64)
        .sum()
}

/// 買い目ポートフォリオの収支シミュレーションを実行する。
pub fn simulate(input: &SimInput) -> Result<SimReport> {
    let field = &input.field;
    let runners = field.len();
    if runners < 3 {
        return Err(Error::OutOfRange(format!(
            "field requires at least 3 horses, got {runners}"
        )));
    }
    {
        // 出走馬の重複チェック。
        let mut seen = std::collections::HashSet::new();
        for h in field {
            if !seen.insert(h.value()) {
                return Err(Error::InvalidFormat(format!(
                    "field has duplicate horse {}",
                    h.value()
                )));
            }
        }
    }

    let total_stake: u64 = input.bets.iter().map(|b| b.stake).sum();

    // EV 用の単勝確率マップ（指定時のみ）。
    let win_map: Option<HashMap<u32, f64>> = input.win_probs.as_ref().map(|ps| {
        ps.iter()
            .map(|p| (p.horse_num.value(), p.win_prob))
            .collect()
    });
    let win_of = |h: HorseNum| -> f64 {
        win_map
            .as_ref()
            .and_then(|m| m.get(&h.value()).copied())
            .unwrap_or(0.0)
    };

    let mut total_count: u64 = 0;
    let mut hit_count: u64 = 0;
    let mut best: Option<Outcome> = None;
    let mut worst_hit: Option<Outcome> = None;
    let mut ev_sum = 0.0_f64;
    let mut hit_prob = 0.0_f64;

    let to_outcome = |first: HorseNum, second: HorseNum, third: HorseNum, payout: u64| Outcome {
        finish: (first, second, third),
        payout,
        pnl: payout as i128 - total_stake as i128,
    };

    for i in 0..runners {
        for j in 0..runners {
            if j == i {
                continue;
            }
            for k in 0..runners {
                if k == i || k == j {
                    continue;
                }
                let (first, second, third) = (field[i], field[j], field[k]);
                total_count += 1;
                let payout = payout_of(&input.bets, first, second, third, runners);

                if payout > 0 {
                    hit_count += 1;
                    if worst_hit
                        .as_ref()
                        .map(|w| payout < w.payout)
                        .unwrap_or(true)
                    {
                        worst_hit = Some(to_outcome(first, second, third, payout));
                    }
                }
                if best.as_ref().map(|b| payout > b.payout).unwrap_or(true) {
                    best = Some(to_outcome(first, second, third, payout));
                }

                if win_map.is_some() {
                    let prob = harville_trifecta(win_of(first), win_of(second), win_of(third));
                    ev_sum += prob * payout as f64;
                    if payout > 0 {
                        hit_prob += prob;
                    }
                }
            }
        }
    }

    // best は最低 1 件列挙されるので必ず存在する（runners >= 3）。
    let best = best.expect("at least one finishing order enumerated");

    let main = match input.main {
        Some((a, b, c)) => {
            if a == b || a == c || b == c {
                return Err(Error::InvalidFormat(
                    "main finish requires three distinct horses".to_string(),
                ));
            }
            let payout = payout_of(&input.bets, a, b, c, runners);
            Some(to_outcome(a, b, c, payout))
        }
        None => None,
    };

    let ev = win_map.as_ref().map(|_| EvReport {
        ev: ev_sum,
        roi: if total_stake > 0 {
            ev_sum / total_stake as f64
        } else {
            0.0
        },
        hit_prob,
    });

    Ok(SimReport {
        total_stake,
        total_count,
        hit_count,
        best,
        worst_hit,
        main,
        ev,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horse_result::HorseName;
    use crate::odds::{Pair, Triple};

    fn h(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    fn field(n: u32) -> Vec<HorseNum> {
        (1..=n).map(h).collect()
    }

    fn bet(combination: BetCombination, stake: u64, odds: f64) -> PlacedBet {
        PlacedBet {
            combination,
            stake,
            odds,
        }
    }

    fn prob(n: u32, win: f64) -> HorseProbability {
        HorseProbability {
            horse_num: h(n),
            horse_name: HorseName::try_from(format!("ウマ{n}")).unwrap(),
            win_prob: win,
            place_prob: 0.0,
            show_prob: 0.0,
        }
    }

    #[test]
    fn win_hit_rule() {
        assert!(is_hit(&BetCombination::Win(h(3)), h(3), h(1), h(2), 10));
        assert!(!is_hit(&BetCombination::Win(h(3)), h(1), h(3), h(2), 10));
    }

    #[test]
    fn place_rule_depends_on_field_size() {
        let c = BetCombination::Place(h(3));
        // 8 頭立て: 3 着以内で的中。
        assert!(is_hit(&c, h(1), h(2), h(3), 8));
        // 7 頭立て: 3 着では的中しない（2 着以内のみ）。
        assert!(!is_hit(&c, h(1), h(2), h(3), 7));
        assert!(is_hit(&c, h(1), h(3), h(2), 7));
    }

    #[test]
    fn quinella_and_wide_rules() {
        let q = BetCombination::Quinella(Pair::try_from((h(1), h(5))).unwrap());
        let w = BetCombination::Wide(Pair::try_from((h(1), h(5))).unwrap());
        // 1-5 が 1・2 着: 馬連もワイドも的中。
        assert!(is_hit(&q, h(5), h(1), h(2), 12));
        assert!(is_hit(&w, h(5), h(1), h(2), 12));
        // 1 が 1 着・5 が 3 着: 馬連は外れ、ワイドは的中。
        assert!(!is_hit(&q, h(1), h(2), h(5), 12));
        assert!(is_hit(&w, h(1), h(2), h(5), 12));
    }

    #[test]
    fn exacta_trio_trifecta_rules() {
        let e = BetCombination::Exacta(crate::odds::OrderedPair::try_from((h(1), h(5))).unwrap());
        let t = BetCombination::Trio(Triple::try_from((h(1), h(5), h(8))).unwrap());
        let tf =
            BetCombination::Trifecta(crate::odds::OrderedTriple::try_from((h(1), h(5), h(8))).unwrap());
        assert!(is_hit(&e, h(1), h(5), h(2), 12));
        assert!(!is_hit(&e, h(5), h(1), h(2), 12)); // 順序違いは外れ
        assert!(is_hit(&t, h(8), h(1), h(5), 12)); // 順不同で 3 着以内
        assert!(!is_hit(&t, h(1), h(5), h(2), 12));
        assert!(is_hit(&tf, h(1), h(5), h(8), 12));
        assert!(!is_hit(&tf, h(1), h(8), h(5), 12)); // 順序違いは外れ
    }

    #[test]
    fn best_worst_and_hit_count() {
        // 6 頭立て。ワイド 1-5 (odds 3.0, 500円) と 三連単 1>5>8 (odds 50.0, 100円)。
        let input = SimInput {
            field: field(6),
            bets: vec![
                bet(BetCombination::Wide(Pair::try_from((h(1), h(5))).unwrap()), 500, 3.0),
                bet(
                    BetCombination::Trifecta(
                        crate::odds::OrderedTriple::try_from((h(1), h(5), h(8))).unwrap(),
                    ),
                    100,
                    50.0,
                ),
            ],
            main: Some((h(1), h(5), h(2))),
            win_probs: None,
        };
        // 8 番は 6 頭立てに居ないので三連単は決して的中しない。
        let r = simulate(&input).unwrap();
        assert_eq!(r.total_stake, 600);
        assert_eq!(r.total_count, 6 * 5 * 4);
        // ベストはワイド的中のみ = 500*3 = 1500。
        assert_eq!(r.best.payout, 1500);
        // 的中する着順は必ずワイド 1500 円のみ（三連単は当たらない）。
        let worst = r.worst_hit.unwrap();
        assert_eq!(worst.payout, 1500);
        assert!(worst.pnl > 0); // 1500 - 600 > 0
        // 本線 1-5-2: 1 と 5 がともに 3 着以内なのでワイド的中。
        assert_eq!(r.main.unwrap().payout, 1500);
        assert!(r.hit_count > 0);
        assert!(r.ev.is_none());
    }

    #[test]
    fn win_but_loss_is_detected() {
        // ワイド 1-5 を 1000 円・低オッズ 1.2 倍 → 払戻 1200。
        // 加えて当たらない単勝 9（6 頭立てに居ない）に 5000 円。総賭け金 6000。
        // 的中してもワイドのみ 1200 < 6000 → 当たっても赤字。
        let input = SimInput {
            field: field(6),
            bets: vec![
                bet(BetCombination::Wide(Pair::try_from((h(1), h(5))).unwrap()), 1000, 1.2),
                bet(BetCombination::Win(h(9)), 5000, 2.0),
            ],
            main: None,
            win_probs: None,
        };
        let r = simulate(&input).unwrap();
        let worst = r.worst_hit.unwrap();
        assert_eq!(worst.payout, 1200);
        assert!(worst.pnl < 0, "的中しても赤字のはず");
    }

    #[test]
    fn ev_with_probabilities() {
        // 3 頭立て、単勝 1（odds 2.0, 1000円）。win_probs: 1=0.5。
        // 着順は 1>2>3, 1>3>2, 2>1>3, 2>3>1, 3>1>2, 3>2>1 の 6 通り。
        // 単勝 1 が的中するのは 1 着が 1 の 2 通り。各 payout=2000。
        // harville_trifecta の確率和で EV を算出。1 着が 1 の確率 = win_prob(1) = 0.5。
        let input = SimInput {
            field: field(3),
            bets: vec![bet(BetCombination::Win(h(1)), 1000, 2.0)],
            main: None,
            win_probs: Some(vec![prob(1, 0.5), prob(2, 0.3), prob(3, 0.2)]),
        };
        let r = simulate(&input).unwrap();
        let ev = r.ev.unwrap();
        // 的中確率 ≈ P(1 着が 1) = 0.5。
        assert!((ev.hit_prob - 0.5).abs() < 1e-9, "hit_prob={}", ev.hit_prob);
        // EV = 0.5 * 2000 = 1000。期待回収率 = 1000/1000 = 1.0。
        assert!((ev.ev - 1000.0).abs() < 1e-6, "ev={}", ev.ev);
        assert!((ev.roi - 1.0).abs() < 1e-9, "roi={}", ev.roi);
    }

    #[test]
    fn too_few_horses_errors() {
        let input = SimInput {
            field: field(2),
            bets: vec![],
            main: None,
            win_probs: None,
        };
        assert!(simulate(&input).is_err());
    }
}
