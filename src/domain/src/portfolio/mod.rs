//! 予算内・軸流しポートフォリオ生成器（純粋ロジック・IO なし, #122）。
//!
//! 予想本命を軸に相手へ流す「馬連＋ワイド＋三連複」を、1 レース予算内・100 円単位で組み立てる。
//! 買い方メモ（軸＝本命を外さない・相手広く・保険のワイド・100 円単位）を encode する。
//! 期待回収率・的中確率は単一レース収支シミュレータ [`simulate`] で評価する（select_bets の
//! EV 羅列を置き換える predict 本番の買い目生成器）。

use std::collections::HashMap;

use crate::betting::BetCombination;
use crate::horse_result::HorseNum;
use crate::odds::{Pair, RaceOdds, Triple};
use crate::prediction::HorseProbability;
use crate::simulation::{EvReport, PlacedBet, SimInput, simulate};

/// ポートフォリオ生成の方針。
#[derive(Debug, Clone)]
pub struct PortfolioConfig {
    /// 相手頭数（軸を除く `win_prob` 上位 N 頭に流す。相手広く）。
    pub partners: usize,
    /// 予算配分の相対重み `(馬連, ワイド, 三連複)`。
    pub alloc: (u32, u32, u32),
}

impl Default for PortfolioConfig {
    /// 既定。相手 5 頭・馬連:ワイド:三連複 = 1:1:1。
    /// 既定値は PR1 の戦略評価ハーネス（`scripts/predict-check/strategy_eval.py`）で
    /// 後追い検証・調整する（相手頭数・配分の感度は同ハーネスで出せる）。
    fn default() -> Self {
        Self {
            partners: 5,
            alloc: (1, 1, 1),
        }
    }
}

/// ポートフォリオ内の 1 買い目。
#[derive(Debug, Clone)]
pub struct PortfolioBet {
    pub combination: BetCombination,
    /// 賭け金（円, 100 円単位）。
    pub stake: u64,
    /// 払戻倍率（ライブ未取得なら `None`。買い目は残し精算は確定配当で可能）。
    /// ワイドは下限..上限帯の中点を用いる。
    pub odds: Option<f64>,
    /// 期待値倍率 = 的中確率 × odds（`simulate` 単体評価の期待回収率）。odds 未取得なら 0.0。
    pub ev: f64,
}

/// 生成された軸流しポートフォリオ。
#[derive(Debug, Clone)]
pub struct Portfolio {
    /// 軸（予想本命）。出走確率が空なら `None`。
    pub axis: Option<HorseNum>,
    /// 相手（流す先）。
    pub partners: Vec<HorseNum>,
    pub bets: Vec<PortfolioBet>,
    pub total_stake: u64,
    /// ポートフォリオ全体の期待回収率・的中確率（`simulate` による。買い目が空なら `None`）。
    pub ev: Option<EvReport>,
}

/// 予想本命を軸に、予算内・100 円単位の軸流しポートフォリオ（馬連＋ワイド＋三連複）を組む。
///
/// 軸 = `win_prob` 最大の馬、相手 = 次点 `config.partners` 頭。馬連・ワイドは軸-相手の K 点、
/// 三連複は軸 1 頭ながし formation（軸＋相手 2 頭, C(K,2) 点）。`config.alloc` の重みで券種へ
/// 予算を配分し、券種内は 100 円単位で均等配分する（賄えない端数は買わない）。
pub fn build_portfolio(
    probs: &[HorseProbability],
    odds: &RaceOdds,
    race_budget: u64,
    config: &PortfolioConfig,
) -> Portfolio {
    // win_prob 降順（同率は馬番昇順）で軸・相手を選ぶ。
    let mut ranked: Vec<&HorseProbability> = probs.iter().collect();
    ranked.sort_by(|a, b| {
        b.win_prob
            .partial_cmp(&a.win_prob)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.horse_num.value().cmp(&b.horse_num.value()))
    });

    let Some((axis_hp, rest)) = ranked.split_first() else {
        return Portfolio {
            axis: None,
            partners: Vec::new(),
            bets: Vec::new(),
            total_stake: 0,
            ev: None,
        };
    };
    let axis = axis_hp.horse_num;
    let partners: Vec<HorseNum> = rest
        .iter()
        .take(config.partners)
        .map(|p| p.horse_num)
        .collect();

    let win: HashMap<HorseNum, f64> = probs.iter().map(|p| (p.horse_num, p.win_prob)).collect();
    let field: Vec<HorseNum> = probs.iter().map(|p| p.horse_num).collect();

    // 券種ごとの脚（組合せ, odds）を生成する。
    // 馬連: 軸-相手 / ワイド: 軸-相手（保険, 帯の中点）
    let quinella: Vec<(BetCombination, Option<f64>)> = partners
        .iter()
        .filter_map(|&p| {
            Pair::try_from((axis, p)).ok().map(|pair| {
                let o = odds.quinella.get(&pair).map(|v| v.value());
                (BetCombination::Quinella(pair), o)
            })
        })
        .collect();
    let wide: Vec<(BetCombination, Option<f64>)> = partners
        .iter()
        .filter_map(|&p| {
            Pair::try_from((axis, p)).ok().map(|pair| {
                let o = odds
                    .wide
                    .get(&pair)
                    .map(|b| (b.low.value() + b.high.value()) / 2.0);
                (BetCombination::Wide(pair), o)
            })
        })
        .collect();
    // 三連複: 軸 + 相手 2 頭（C(K,2) 点）。
    let mut trio: Vec<(BetCombination, Option<f64>)> = Vec::new();
    for i in 0..partners.len() {
        for j in (i + 1)..partners.len() {
            if let Ok(t) = Triple::try_from((axis, partners[i], partners[j])) {
                let o = odds.trio.get(&t).map(|v| v.value());
                trio.push((BetCombination::Trio(t), o));
            }
        }
    }

    // 予算配分（重み→券種予算、券種内 100 円単位均等配分）。
    let (wq, ww, wt) = config.alloc;
    let total_w = (wq + ww + wt) as u128;
    let mut bets: Vec<PortfolioBet> = Vec::new();
    if total_w > 0 {
        let type_budget =
            |w: u32| -> u64 { (race_budget as u128 * w as u128 / total_w / 100 * 100) as u64 };
        push_legs(&mut bets, quinella, type_budget(wq), &field, &win);
        push_legs(&mut bets, wide, type_budget(ww), &field, &win);
        push_legs(&mut bets, trio, type_budget(wt), &field, &win);
    }

    let total_stake: u64 = bets.iter().map(|b| b.stake).sum();

    // ポートフォリオ全体の期待回収率・的中確率。odds 未取得の脚は払戻を見積もれないため
    // EV 評価から除外する（odds=0 で混ぜると的中 0 の stake が分母 total_stake を膨らませ ROI を
    // 過小評価するため）。よって roi は「オッズ取得済みの脚」についての期待回収率になる。
    let priced: Vec<PlacedBet> = bets
        .iter()
        .filter_map(|b| {
            b.odds.map(|o| PlacedBet {
                combination: b.combination.clone(),
                stake: b.stake,
                odds: o,
            })
        })
        .collect();
    let ev = if priced.is_empty() || field.len() < 3 {
        None
    } else {
        simulate(&SimInput {
            field: field.clone(),
            bets: priced,
            main: None,
            win_probs: Some(win.clone()),
        })
        .ok()
        .and_then(|r| r.ev)
    };

    Portfolio {
        axis: Some(axis),
        partners,
        bets,
        total_stake,
        ev,
    }
}

/// 1 券種ぶんの脚を予算配分して `out` に積む。`type_budget` を 100 円単位で均等配分し、
/// 賭け金 0 の脚は捨てる。各脚の ev 倍率は `simulate` 単体評価の期待回収率で求める。
fn push_legs(
    out: &mut Vec<PortfolioBet>,
    legs: Vec<(BetCombination, Option<f64>)>,
    type_budget: u64,
    field: &[HorseNum],
    win: &HashMap<HorseNum, f64>,
) {
    let stakes = distribute(type_budget, legs.len());
    for ((combination, odds), stake) in legs.into_iter().zip(stakes) {
        if stake == 0 {
            continue;
        }
        let ev = leg_ev(field, win, &combination, odds);
        out.push(PortfolioBet {
            combination,
            stake,
            odds,
            ev,
        });
    }
}

/// `type_budget`（100 円単位前提）を `n` 点に 100 円単位で均等配分する。
/// 全点 100 円すら賄えないなら、賄える点数ぶんだけ 100 円ずつ張る（残りは 0＝買わない）。
/// PR1 ハーネス `strategy_eval.distribute` と同じ流儀。
fn distribute(type_budget: u64, n: usize) -> Vec<u64> {
    if n == 0 || type_budget < 100 {
        return vec![0; n];
    }
    let per = type_budget / n as u64 / 100 * 100;
    if per >= 100 {
        return vec![per; n];
    }
    let affordable = (type_budget / 100) as usize;
    let mut v = vec![100u64; affordable.min(n)];
    v.resize(n, 0);
    v
}

/// 脚 1 点の期待値倍率（= 的中確率 × odds）を `simulate` 単体評価で求める。
/// ワイドのように的中確率の閉形式が無い券種も、着順列挙の収支シミュレータに委譲して正確に出す
/// （`simulate` の roi = E\[払戻\]/賭け金 = 的中確率 × odds）。odds 未取得・頭数不足は 0.0。
fn leg_ev(
    field: &[HorseNum],
    win: &HashMap<HorseNum, f64>,
    combination: &BetCombination,
    odds: Option<f64>,
) -> f64 {
    let Some(o) = odds else {
        return 0.0;
    };
    if field.len() < 3 {
        return 0.0;
    }
    simulate(&SimInput {
        field: field.to_vec(),
        bets: vec![PlacedBet {
            combination: combination.clone(),
            stake: 100,
            odds: o,
        }],
        main: None,
        win_probs: Some(win.clone()),
    })
    .ok()
    .and_then(|r| r.ev)
    .map(|e| e.roi)
    .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horse_result::HorseName;
    use crate::odds::{OddsValue, PlaceOdds};
    use crate::race::RaceId;

    fn horse(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    fn prob(n: u32, win: f64) -> HorseProbability {
        HorseProbability {
            horse_num: horse(n),
            horse_name: HorseName::try_from(format!("ウマ{n}")).unwrap(),
            win_prob: win,
            place_prob: 0.0,
            show_prob: 0.0,
        }
    }

    fn odds(v: f64) -> OddsValue {
        OddsValue::try_from(v).unwrap()
    }

    /// 軸 = 馬1（最高 win_prob）、相手 = 馬2,3,4。全券種オッズあり。
    fn sample() -> (Vec<HorseProbability>, RaceOdds) {
        let probs = vec![
            prob(1, 0.40),
            prob(2, 0.25),
            prob(3, 0.18),
            prob(4, 0.10),
            prob(5, 0.07),
        ];
        let mut o = RaceOdds::empty(RaceId::try_from("202506040101".to_string()).unwrap());
        for p in 2..=5 {
            o.quinella
                .insert(Pair::try_from((horse(1), horse(p))).unwrap(), odds(5.0));
            o.wide.insert(
                Pair::try_from((horse(1), horse(p))).unwrap(),
                PlaceOdds::try_from((odds(2.0), odds(3.0))).unwrap(),
            );
        }
        for i in 2..=5 {
            for j in (i + 1)..=5 {
                o.trio.insert(
                    Triple::try_from((horse(1), horse(i), horse(j))).unwrap(),
                    odds(20.0),
                );
            }
        }
        (probs, o)
    }

    #[test]
    fn selects_axis_and_partners_by_win_prob() {
        let (probs, o) = sample();
        let config = PortfolioConfig {
            partners: 3,
            alloc: (1, 1, 1),
        };
        let pf = build_portfolio(&probs, &o, 5000, &config);
        assert_eq!(pf.axis, Some(horse(1)));
        assert_eq!(pf.partners, vec![horse(2), horse(3), horse(4)]);
    }

    #[test]
    fn generates_axis_nagashi_legs_within_budget_in_100_units() {
        let (probs, o) = sample();
        let config = PortfolioConfig {
            partners: 3,
            alloc: (1, 1, 1),
        };
        let pf = build_portfolio(&probs, &o, 5000, &config);

        let count = |label: &str| {
            pf.bets
                .iter()
                .filter(|b| b.combination.type_label() == label)
                .count()
        };
        // 馬連・ワイドは相手 3 点ずつ、三連複は C(3,2)=3 点。
        assert_eq!(count("quinella"), 3);
        assert_eq!(count("wide"), 3);
        assert_eq!(count("trio"), 3);
        // すべて 100 円単位、合計は予算以内。
        assert!(pf.bets.iter().all(|b| b.stake % 100 == 0 && b.stake > 0));
        assert!(pf.total_stake <= 5000, "stake {} <= 5000", pf.total_stake);
        // EV（期待回収率）が算出される。
        assert!(pf.ev.is_some());
    }

    #[test]
    fn trio_needs_two_partners() {
        let (probs, o) = sample();
        let config = PortfolioConfig {
            partners: 1,
            alloc: (1, 1, 1),
        };
        let pf = build_portfolio(&probs, &o, 5000, &config);
        // 相手 1 頭では三連複は組めない（C(1,2)=0）。馬連・ワイドは 1 点ずつ。
        assert!(
            !pf.bets
                .iter()
                .any(|b| b.combination.type_label() == "trio")
        );
        assert_eq!(pf.partners, vec![horse(2)]);
    }

    #[test]
    fn missing_odds_keeps_bet_with_zero_ev() {
        // 三連複オッズを空にする → 三連複の脚は残るが ev=0。
        let (probs, mut o) = sample();
        o.trio.clear();
        let config = PortfolioConfig {
            partners: 3,
            alloc: (1, 1, 1),
        };
        let pf = build_portfolio(&probs, &o, 5000, &config);
        let trio: Vec<_> = pf
            .bets
            .iter()
            .filter(|b| b.combination.type_label() == "trio")
            .collect();
        assert_eq!(trio.len(), 3);
        assert!(trio.iter().all(|b| b.odds.is_none() && b.ev == 0.0));
    }

    #[test]
    fn default_config_stays_within_budget() {
        // 既定 PortfolioConfig（相手 5・配分 1:1:1）でも 100 円単位・予算以内に収まる。
        let (probs, o) = sample();
        let pf = build_portfolio(&probs, &o, 5000, &PortfolioConfig::default());
        assert!(pf.bets.iter().all(|b| b.stake % 100 == 0 && b.stake > 0));
        assert!(pf.total_stake <= 5000, "stake {} <= 5000", pf.total_stake);
        // 相手 4 頭（馬5まで）→ 馬連4・ワイド4・三連複 C(4,2)=6。
        assert_eq!(pf.partners.len(), 4);
    }

    #[test]
    fn portfolio_roi_excludes_unpriced_legs() {
        // ワイドのオッズを空にすると、ワイド脚は ROI 評価から除外され、
        // ポートフォリオ ROI は「オッズ取得済みの脚（馬連＋三連複）」だけで算出される。
        let (probs, mut o) = sample();
        o.wide.clear();
        let config = PortfolioConfig {
            partners: 3,
            alloc: (1, 1, 1),
        };
        let pf = build_portfolio(&probs, &o, 5000, &config);
        let ev = pf.ev.expect("ev should be Some when priced legs exist");

        // 参照: オッズ取得済みの脚だけを simulate した期待回収率と一致すること。
        let field: Vec<HorseNum> = probs.iter().map(|p| p.horse_num).collect();
        let win: std::collections::HashMap<HorseNum, f64> =
            probs.iter().map(|p| (p.horse_num, p.win_prob)).collect();
        let priced: Vec<PlacedBet> = pf
            .bets
            .iter()
            .filter_map(|b| {
                b.odds.map(|odds| PlacedBet {
                    combination: b.combination.clone(),
                    stake: b.stake,
                    odds,
                })
            })
            .collect();
        // ワイド 3 点は未取得、馬連 3＋三連複 3 が priced。
        assert_eq!(priced.len(), 6);
        let reference = simulate(&SimInput {
            field,
            bets: priced,
            main: None,
            win_probs: Some(win),
        })
        .unwrap()
        .ev
        .unwrap();
        assert!(
            (ev.roi - reference.roi).abs() < 1e-9,
            "roi {} should match priced-only {}",
            ev.roi,
            reference.roi
        );
    }

    #[test]
    fn empty_probabilities_is_empty_portfolio() {
        let o = RaceOdds::empty(RaceId::try_from("202506040101".to_string()).unwrap());
        let pf = build_portfolio(&[], &o, 5000, &PortfolioConfig::default());
        assert_eq!(pf.axis, None);
        assert!(pf.bets.is_empty());
        assert!(pf.ev.is_none());
    }

    #[test]
    fn budget_too_small_buys_what_it_can() {
        let (probs, o) = sample();
        // 予算 200 円・配分 1:0:0（馬連のみ）・相手 3 → 馬連 3 点に 200 円では 2 点だけ 100 円。
        let config = PortfolioConfig {
            partners: 3,
            alloc: (1, 0, 0),
        };
        let pf = build_portfolio(&probs, &o, 200, &config);
        let quinella: Vec<_> = pf
            .bets
            .iter()
            .filter(|b| b.combination.type_label() == "quinella")
            .collect();
        assert_eq!(quinella.len(), 2, "200 円では 100 円 × 2 点まで");
        assert!(pf.total_stake <= 200);
        // ワイド・三連複は配分 0 なので無し。
        assert!(pf.bets.iter().all(|b| b.combination.type_label() == "quinella"));
    }
}
