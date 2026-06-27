//! 予算内・軸流しポートフォリオ生成器（純粋ロジック・IO なし, #122）。
//!
//! 予想本命を軸に相手へ流す「馬連＋ワイド＋三連複」を、1 レース予算内・100 円単位で組み立てる。
//! 買い方メモ（軸＝本命を外さない・相手広く・保険のワイド・100 円単位）を encode する。
//! 期待回収率・的中確率は単一レース収支シミュレータ [`simulate`] で評価する（select_bets の
//! EV 羅列を置き換える predict 本番の買い目生成器）。

use std::collections::HashMap;

use crate::betting::BetCombination;
use crate::horse_result::HorseNum;
use crate::odds::{OrderedPair, Pair, RaceOdds, Triple};
use crate::prediction::HorseProbability;
use crate::simulation::{EvReport, PlacedBet, SimInput, simulate};

/// ポートフォリオ生成の方針。
#[derive(Debug, Clone)]
pub struct PortfolioConfig {
    /// 相手頭数（軸を除く `win_prob` 上位 N 頭に流す。相手広く）。
    pub partners: usize,
    /// 予算配分の相対重み `(連系ペア, ワイド, 三連複)`。連系ペアは馬連/馬単のうち EV 優位な方（#246）。
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

/// win_prob 降順（同率は馬番昇順）で軸＝本命と相手上位 `partners` 頭を選ぶ。出走確率が空なら `None`。
/// `build_portfolio` と `pair_ev_diagnostics` で共用し、軸・相手の決め方を一元化する。
fn rank_axis_partners(
    probs: &[HorseProbability],
    partners: usize,
) -> Option<(HorseNum, Vec<HorseNum>)> {
    let mut ranked: Vec<&HorseProbability> = probs.iter().collect();
    ranked.sort_by(|a, b| {
        b.win_prob
            .partial_cmp(&a.win_prob)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.horse_num.value().cmp(&b.horse_num.value()))
    });
    let (axis_hp, rest) = ranked.split_first()?;
    let ps = rest.iter().take(partners).map(|p| p.horse_num).collect();
    Some((axis_hp.horse_num, ps))
}

/// 軸-相手 1 ペアの「馬連 vs 馬単(両方向)」EV 診断 1 行（#246-C）。
/// EV は `leg_ev`（収支シミュレータ）で算出し、odds 未取得は `None`（EV 0.0）。
#[derive(Debug, Clone)]
pub struct PairEvDiagnostic {
    pub partner: HorseNum,
    pub quinella_ev: f64,
    pub quinella_odds: Option<f64>,
    /// 馬単 軸→相手（本命→相手）。
    pub exacta_fwd_ev: f64,
    pub exacta_fwd_odds: Option<f64>,
    /// 馬単 相手→軸（相手→本命）。
    pub exacta_rev_ev: f64,
    pub exacta_rev_odds: Option<f64>,
}

/// 軸（win_prob 最大）と相手上位 `partners` 頭について、各ペアの馬連・馬単(両方向) EV を並べる
/// 診断（#246-C）。手作業でオッズを引いて比較していた「馬連 vs 馬単」の判断材料を CLI に出すための
/// 純粋関数。EV 計算は `build_portfolio` と同じ `leg_ev`（simulate 委譲）に揃える。出走確率が空なら
/// 軸 `None`・空ベクタを返す。
pub fn pair_ev_diagnostics(
    probs: &[HorseProbability],
    odds: &RaceOdds,
    partners: usize,
) -> (Option<HorseNum>, Vec<PairEvDiagnostic>) {
    let Some((axis, partner_nums)) = rank_axis_partners(probs, partners) else {
        return (None, Vec::new());
    };
    let win: HashMap<HorseNum, f64> = probs.iter().map(|p| (p.horse_num, p.win_prob)).collect();
    let field: Vec<HorseNum> = probs.iter().map(|p| p.horse_num).collect();

    let rows = partner_nums
        .iter()
        .map(|&p| {
            let quinella_odds = Pair::try_from((axis, p))
                .ok()
                .and_then(|pair| odds.quinella.get(&pair).map(|v| v.value()));
            let exacta_fwd_odds = OrderedPair::try_from((axis, p))
                .ok()
                .and_then(|op| odds.exacta.get(&op).map(|v| v.value()));
            let exacta_rev_odds = OrderedPair::try_from((p, axis))
                .ok()
                .and_then(|op| odds.exacta.get(&op).map(|v| v.value()));
            let quinella_ev = Pair::try_from((axis, p))
                .ok()
                .map(|pair| leg_ev(&field, &win, &BetCombination::Quinella(pair), quinella_odds))
                .unwrap_or(0.0);
            let exacta_fwd_ev = OrderedPair::try_from((axis, p))
                .ok()
                .map(|op| leg_ev(&field, &win, &BetCombination::Exacta(op), exacta_fwd_odds))
                .unwrap_or(0.0);
            let exacta_rev_ev = OrderedPair::try_from((p, axis))
                .ok()
                .map(|op| leg_ev(&field, &win, &BetCombination::Exacta(op), exacta_rev_odds))
                .unwrap_or(0.0);
            PairEvDiagnostic {
                partner: p,
                quinella_ev,
                quinella_odds,
                exacta_fwd_ev,
                exacta_fwd_odds,
                exacta_rev_ev,
                exacta_rev_odds,
            }
        })
        .collect();
    (Some(axis), rows)
}

/// 予想本命を軸に、予算内・100 円単位の軸流しポートフォリオ（連系ペア＋ワイド＋三連複）を組む。
///
/// 軸 = `win_prob` 最大の馬、相手 = 次点 `config.partners` 頭。連系ペアは軸-相手ごとに馬連と馬単
/// (本命→相手) の実効 EV を比べ優位な方を 1 脚採用（#246, [`pick_pair_leg`]）、ワイドは軸-相手の
/// K 点、三連複は軸 1 頭ながし formation（軸＋相手 2 頭, C(K,2) 点）。`config.alloc` の重み
/// `(連系ペア, ワイド, 三連複)` で券種へ予算を配分し、券種内は 100 円単位で均等配分する
/// （賄えない端数は買わない）。
pub fn build_portfolio(
    probs: &[HorseProbability],
    odds: &RaceOdds,
    race_budget: u64,
    config: &PortfolioConfig,
) -> Portfolio {
    // win_prob 降順（同率は馬番昇順）で軸・相手を選ぶ。
    let Some((axis, partners)) = rank_axis_partners(probs, config.partners) else {
        return Portfolio {
            axis: None,
            partners: Vec::new(),
            bets: Vec::new(),
            total_stake: 0,
            ev: None,
        };
    };

    let win: HashMap<HorseNum, f64> = probs.iter().map(|p| (p.horse_num, p.win_prob)).collect();
    let field: Vec<HorseNum> = probs.iter().map(|p| p.horse_num).collect();

    // 券種ごとの脚（組合せ, odds）を生成する。
    // 連系ペア: 軸-相手ごとに馬連 vs 馬単(本命→相手) を実効EVで比較し優位な方を採る（#246）。
    // ワイド: 軸-相手（保険, 帯の中点）
    let pair_legs: Vec<(BetCombination, Option<f64>)> = partners
        .iter()
        .filter_map(|&p| pick_pair_leg(axis, p, odds, &field, &win))
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
        // 重み w の券種予算を 100 円単位に floor する（`/100*100`）。乗算オーバーフロー回避に u128。
        let type_budget =
            |w: u32| -> u64 { (race_budget as u128 * w as u128 / total_w / 100 * 100) as u64 };
        push_legs(&mut bets, pair_legs, type_budget(wq), &field, &win);
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

/// 軸-相手 1 ペアの連系脚を選ぶ（#246）。馬連 `Quinella(軸,相手)` と馬単 `Exacta(軸→相手)` の
/// 実効 EV（`leg_ev` = 的中確率 × odds）を比べ、**両方のオッズが揃い馬単が strict に上回るときだけ**
/// 馬単を採る。tie・どちらか欠落は着順不問の馬連を維持する（"常に馬単優先にしない" の担保）。
/// 穴の 1 着確率が小さいほど `harville_exacta(軸→穴)` が相対的に有利化し、(A) の win 校正と連動する。
/// 馬連 `Pair` が作れない（軸==相手, 通常起きない）ときは `None` で脚をスキップする。
fn pick_pair_leg(
    axis: HorseNum,
    partner: HorseNum,
    odds: &RaceOdds,
    field: &[HorseNum],
    win: &HashMap<HorseNum, f64>,
) -> Option<(BetCombination, Option<f64>)> {
    let pair = Pair::try_from((axis, partner)).ok()?;
    let q_odds = odds.quinella.get(&pair).map(|v| v.value());
    let quinella_leg = (BetCombination::Quinella(pair), q_odds);

    // 馬単 軸→相手。OrderedPair が作れない/オッズ欠落/馬連オッズ欠落のときは馬連を維持。
    let Ok(ord) = OrderedPair::try_from((axis, partner)) else {
        return Some(quinella_leg);
    };
    let e_odds = odds.exacta.get(&ord).map(|v| v.value());
    match (q_odds, e_odds) {
        (Some(_), Some(_)) => {
            let q_ev = leg_ev(field, win, &quinella_leg.0, q_odds);
            let e_ev = leg_ev(field, win, &BetCombination::Exacta(ord), e_odds);
            if e_ev > q_ev {
                Some((BetCombination::Exacta(ord), e_odds))
            } else {
                Some(quinella_leg)
            }
        }
        _ => Some(quinella_leg),
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
        assert!(!pf.bets.iter().any(|b| b.combination.type_label() == "trio"));
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
        assert!(
            pf.bets
                .iter()
                .all(|b| b.combination.type_label() == "quinella")
        );
    }

    #[test]
    fn exacta_chosen_when_ev_beats_quinella() {
        // 軸1→相手2 の馬単に高オッズを与え、馬連(5.0)より実効EVが上回る → その脚は馬単になる。
        // 他ペア（exacta オッズ無し）は馬連のまま。
        let (probs, mut o) = sample();
        o.exacta.insert(
            OrderedPair::try_from((horse(1), horse(2))).unwrap(),
            odds(100.0),
        );
        let config = PortfolioConfig {
            partners: 3,
            alloc: (1, 0, 0), // 連系ペアのみに絞って検証
        };
        let pf = build_portfolio(&probs, &o, 5000, &config);
        let has_exacta_1_2 = pf.bets.iter().any(|b| {
            matches!(&b.combination, BetCombination::Exacta(op)
                if op.as_tuple() == (horse(1), horse(2)))
        });
        assert!(has_exacta_1_2, "高オッズ馬単 1→2 が選ばれる: {:?}", pf.bets);
        // 相手3,4 は exacta 無し → 馬連のまま。連系ペアは計3点（馬連2 + 馬単1）。
        let pair_legs = pf
            .bets
            .iter()
            .filter(|b| {
                matches!(
                    b.combination,
                    BetCombination::Quinella(_) | BetCombination::Exacta(_)
                )
            })
            .count();
        assert_eq!(pair_legs, 3);
        assert_eq!(
            pf.bets
                .iter()
                .filter(|b| matches!(b.combination, BetCombination::Quinella(_)))
                .count(),
            2
        );
    }

    #[test]
    fn quinella_kept_when_exacta_lower_ev() {
        // 馬単オッズが低い（1.1）→ 馬連(5.0)が EV 優位 → 馬連のまま。
        let (probs, mut o) = sample();
        o.exacta.insert(
            OrderedPair::try_from((horse(1), horse(2))).unwrap(),
            odds(1.1),
        );
        let config = PortfolioConfig {
            partners: 3,
            alloc: (1, 0, 0),
        };
        let pf = build_portfolio(&probs, &o, 5000, &config);
        assert!(
            !pf.bets
                .iter()
                .any(|b| matches!(b.combination, BetCombination::Exacta(_))),
            "低EV馬単は採用されない: {:?}",
            pf.bets
        );
        assert_eq!(
            pf.bets
                .iter()
                .filter(|b| matches!(b.combination, BetCombination::Quinella(_)))
                .count(),
            3
        );
    }

    #[test]
    fn pair_ev_diagnostics_lists_both_directions() {
        let (probs, mut o) = sample();
        // 相手2 に馬単 両方向オッズを入れる。相手3,4 は exacta 無し → fwd/rev odds は None。
        o.exacta.insert(
            OrderedPair::try_from((horse(1), horse(2))).unwrap(),
            odds(40.0),
        );
        o.exacta.insert(
            OrderedPair::try_from((horse(2), horse(1))).unwrap(),
            odds(60.0),
        );
        let (axis, rows) = pair_ev_diagnostics(&probs, &o, 3);
        assert_eq!(axis, Some(horse(1)));
        assert_eq!(rows.len(), 3);
        let r2 = rows.iter().find(|r| r.partner == horse(2)).unwrap();
        // 馬連オッズ(5.0)・馬単両方向オッズが取れている。
        assert_eq!(r2.quinella_odds, Some(5.0));
        assert_eq!(r2.exacta_fwd_odds, Some(40.0));
        assert_eq!(r2.exacta_rev_odds, Some(60.0));
        assert!(r2.quinella_ev > 0.0 && r2.exacta_fwd_ev > 0.0 && r2.exacta_rev_ev > 0.0);
        // 相手3 は exacta 無し → odds None・EV 0。
        let r3 = rows.iter().find(|r| r.partner == horse(3)).unwrap();
        assert_eq!(r3.exacta_fwd_odds, None);
        assert_eq!(r3.exacta_fwd_ev, 0.0);
    }

    #[test]
    fn pair_ev_diagnostics_empty_when_no_probs() {
        let o = RaceOdds::empty(RaceId::try_from("202506040101".to_string()).unwrap());
        let (axis, rows) = pair_ev_diagnostics(&[], &o, 5);
        assert_eq!(axis, None);
        assert!(rows.is_empty());
    }

    #[test]
    fn exacta_swap_preserves_budget_and_units() {
        // 馬単へ swap しても 100 円単位・予算内が保たれる。
        let (probs, mut o) = sample();
        o.exacta.insert(
            OrderedPair::try_from((horse(1), horse(2))).unwrap(),
            odds(100.0),
        );
        let pf = build_portfolio(&probs, &o, 5000, &PortfolioConfig::default());
        assert!(pf.bets.iter().all(|b| b.stake % 100 == 0 && b.stake > 0));
        assert!(pf.total_stake <= 5000, "stake {} <= 5000", pf.total_stake);
    }
}
