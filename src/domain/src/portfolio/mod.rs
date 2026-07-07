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

/// 1 脚の候補 `(組合せ, 払戻倍率)`。odds 未取得は `None`。
type Leg = (BetCombination, Option<f64>);
/// 予算配分の単位となる券種レイヤー `(脚一覧, 相対重み, 方式)`。
type BetLayer = (Vec<Leg>, u32, BetMethod);

/// 買い目の方式。混戦時のみ [`BetMethod::Box`]（印馬3連複ボックス）が現れる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BetMethod {
    /// ◎軸ながし（軸を固定して相手へ流す）。
    Nagashi,
    /// 印馬3連複ボックス（軸なし・混戦時のみ・band 上位の総当たり）。
    Box,
}

/// 混戦時の券種配分の相対重み `(連系ペア, ワイド, 三連複ながし, 印馬3連複ボックス)`。
/// CLAUDE.md「混戦判定と配分」の ¥5,000 基準（馬連¥1,000 / ワイド¥1,000 / 3連複ながし¥1,500 /
/// 3連複ボックス¥1,500）をそのまま相対重みにしたもの。race_budget にスケールして使う。
/// Python `live_ev.py` の `alloc(konsen)=(box1500, wide1000, quinella1000, trio1500)` と等価。
const KONSEN_ALLOC: (u32, u32, u32, u32) = (1000, 1000, 1500, 1500);

/// ポートフォリオ生成の方針。
#[derive(Debug, Clone)]
pub struct PortfolioConfig {
    /// 相手頭数（軸を除く `win_prob` 上位 N 頭に流す。相手広く）。
    pub partners: usize,
    /// 予算配分の相対重み `(連系ペア, ワイド, 三連複)`。連系ペアは常に馬連
    /// （#271 で馬単置換 `pick_pair_leg` を撤去・ADR 0043 棄却）。
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
    /// 方式（◎軸ながし / 印馬3連複ボックス）。box は軸を持たない（`snapshot` が axis=None で出す）。
    pub method: BetMethod,
    /// 賭け金（円, 100 円単位）。
    pub stake: u64,
    /// 払戻倍率（ライブ未取得なら `None`。買い目は残し精算は確定配当で可能）。
    /// ワイドは下限..上限帯の中点を用いる。
    pub odds: Option<f64>,
    /// 期待値倍率 = 的中確率 × floor(100×odds)/100（`leg_metrics` 算出, 払戻 floor 込み）。odds 未取得なら 0.0。
    pub ev: f64,
    /// 的中確率（この組合せが当たる確率, オッズ非依存）。買うか判断する材料として表示する。
    pub hit_prob: f64,
}

/// 生成された軸流しポートフォリオ。
#[derive(Debug, Clone)]
pub struct Portfolio {
    /// 軸（予想本命）。出走確率が空なら `None`。
    pub axis: Option<HorseNum>,
    /// 相手（流す先）。
    pub partners: Vec<HorseNum>,
    /// 混戦（◎の win_prob 0.70 倍以上が ◎含め 4 頭以上）か。混戦時は印馬3連複ボックスを重ねる。
    pub konsen: bool,
    pub bets: Vec<PortfolioBet>,
    pub total_stake: u64,
    /// ポートフォリオ全体の期待回収率・的中確率（`simulate` による。買い目が空なら `None`）。
    pub ev: Option<EvReport>,
}

/// 馬連/馬単の組合せに対応する確定オッズ（払戻倍率）を引く。対象外券種・未取得は `None`。
fn combo_odds(odds: &RaceOdds, combination: &BetCombination) -> Option<f64> {
    match combination {
        BetCombination::Quinella(pair) => odds.quinella.get(pair).map(|v| v.value()),
        BetCombination::Exacta(op) => odds.exacta.get(op).map(|v| v.value()),
        _ => None,
    }
}

/// `rank_probs` と `ev_probs` が同一馬集合かを検査する（`build_portfolio`/`pair_ev_diagnostics` の
/// `debug_assert` 用, #272）。順位付け（rank）と EV 評価（ev）で別系統の確率を渡すため、両者が同じ
/// レースの同じ出走馬を指すことを保証する。順序は問わず馬番の集合一致だけを見る。
fn same_field(rank_probs: &[HorseProbability], ev_probs: &[HorseProbability]) -> bool {
    use std::collections::HashSet;
    let r: HashSet<HorseNum> = rank_probs.iter().map(|p| p.horse_num).collect();
    let e: HashSet<HorseNum> = ev_probs.iter().map(|p| p.horse_num).collect();
    r == e
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

/// 馬連 vs 馬単 EV 診断の結果（#246-C）。`axis`＝本命（出走確率が空なら `None`）、`rows`＝各相手の行。
#[derive(Debug, Clone)]
pub struct PairEvDiagnostics {
    pub axis: Option<HorseNum>,
    pub rows: Vec<PairEvDiagnostic>,
}

/// 軸（win_prob 最大）と相手上位 `partners` 頭について、各ペアの馬連・馬単(両方向) EV を並べる
/// 診断（#246-C）。手作業でオッズを引いて比較していた「馬連 vs 馬単」の判断材料を CLI に出すための
/// 純粋関数。EV 計算は `build_portfolio` と同じ `leg_ev`（simulate 委譲）に揃える。出走確率が空なら
/// 軸 `None`・空ベクタを返す。
pub fn pair_ev_diagnostics(
    rank_probs: &[HorseProbability],
    ev_probs: &[HorseProbability],
    odds: &RaceOdds,
    partners: usize,
) -> PairEvDiagnostics {
    debug_assert!(
        same_field(rank_probs, ev_probs),
        "rank_probs と ev_probs は同一馬集合でなければならない"
    );
    // 軸・相手は rank_probs（市場ブレンド）で選び、EV は ev_probs（純モデル）で評価する（循環断ち, #272）。
    let Some((axis, partner_nums)) = rank_axis_partners(rank_probs, partners) else {
        return PairEvDiagnostics {
            axis: None,
            rows: Vec::new(),
        };
    };
    let win: HashMap<HorseNum, f64> = ev_probs.iter().map(|p| (p.horse_num, p.win_prob)).collect();
    let field: Vec<HorseNum> = ev_probs.iter().map(|p| p.horse_num).collect();

    // 馬連/馬単(両方向) の EV と odds を 1 行に出す。各組合せの odds と EV はペアで求める
    // （odds 取得と EV 計算で try_from を二度呼ばないようヘルパに束ねる）。
    let leg = |combo: BetCombination| {
        let cell_odds = combo_odds(odds, &combo);
        let ev = leg_ev(&field, &win, &combo, cell_odds);
        (ev, cell_odds)
    };
    let rows = partner_nums
        .iter()
        .filter_map(|&p| {
            let pair = Pair::try_from((axis, p)).ok()?;
            let fwd = OrderedPair::try_from((axis, p)).ok()?;
            let rev = OrderedPair::try_from((p, axis)).ok()?;
            let (quinella_ev, quinella_odds) = leg(BetCombination::Quinella(pair));
            let (exacta_fwd_ev, exacta_fwd_odds) = leg(BetCombination::Exacta(fwd));
            let (exacta_rev_ev, exacta_rev_odds) = leg(BetCombination::Exacta(rev));
            Some(PairEvDiagnostic {
                partner: p,
                quinella_ev,
                quinella_odds,
                exacta_fwd_ev,
                exacta_fwd_odds,
                exacta_rev_ev,
                exacta_rev_odds,
            })
        })
        .collect();
    PairEvDiagnostics {
        axis: Some(axis),
        rows,
    }
}

/// ◎（`win_prob` 最大）の 0.70 倍以上の馬（◎含む）を `win_prob` 降順（同率は馬番昇順）で返す＝混戦判定の母集団。
/// Python `live_ev.py:band_of` と等価。軸選定と同じ確率系列（`rank_probs`＝市場ブレンド）を渡すことで
/// `band[0] == axis`（◎）が保たれ、「◎含め」の意味が壊れない。空なら空 Vec。
fn band_of(probs: &[HorseProbability]) -> Vec<HorseNum> {
    if probs.is_empty() {
        return Vec::new();
    }
    let max_win = probs.iter().map(|p| p.win_prob).fold(f64::MIN, f64::max);
    let mut band: Vec<&HorseProbability> = probs
        .iter()
        .filter(|p| p.win_prob >= 0.70 * max_win)
        .collect();
    band.sort_by(|a, b| {
        b.win_prob
            .partial_cmp(&a.win_prob)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.horse_num.value().cmp(&b.horse_num.value()))
    });
    band.into_iter().map(|p| p.horse_num).collect()
}

/// 印馬（band 上位最大 5 頭）の 3 連複ボックス脚（軸なし・C(n,3) 点）を組む。
/// Python `live_ev.py` の `band_of(probs)[:5]` の `combinations(box, 3)` と等価。
fn build_box_legs(band: &[HorseNum], odds: &RaceOdds) -> Vec<(BetCombination, Option<f64>)> {
    let box_horses: Vec<HorseNum> = band.iter().take(5).copied().collect();
    let mut legs = Vec::new();
    for i in 0..box_horses.len() {
        for j in (i + 1)..box_horses.len() {
            for k in (j + 1)..box_horses.len() {
                if let Ok(t) = Triple::try_from((box_horses[i], box_horses[j], box_horses[k])) {
                    let o = odds.trio.get(&t).map(|v| v.value());
                    legs.push((BetCombination::Trio(t), o));
                }
            }
        }
    }
    legs
}

/// 予想本命を軸に、予算内・100 円単位の軸流しポートフォリオ（連系ペア＋ワイド＋三連複）を組む。
///
/// 軸 = `win_prob` 最大の馬、相手 = 次点 `config.partners` 頭。連系ペアは軸-相手の馬連（着順不問）を
/// 常に採る（#271 で馬連→馬単の EV 置換を撤去・ADR 0043 棄却。順序プレミアムは #262 の 71R で逆予測）。
/// ワイドは軸-相手の K 点、三連複は軸 1 頭ながし（軸＋相手 2 頭, C(K,2) 点）。`config.alloc` の重み
/// `(連系ペア, ワイド, 三連複)` で券種へ予算を配分し、券種内は 100 円単位で均等配分する
/// （賄えない端数は買わない）。
///
/// 混戦（`band_of(rank_probs)` が ◎含め 4 頭以上）のときは、上記の券種配分を `KONSEN_ALLOC`
/// （馬連1000/ワイド1000/三連複ながし1500/印馬3連複ボックス1500）に切り替え、印馬（=確率 band 上位
/// 最大 5 頭・[`build_box_legs`]）の 3 連複ボックスレイヤーを重ねる（◎が飛んでも印が揃えば拾う保険）。
/// **混戦時は `config.alloc` を使わず `KONSEN_ALLOC` を用いる**（CLAUDE.md「混戦判定と配分」準拠）。
/// なお「印馬」は Mark（◎○▲☆）ではなく確率 band（◎の 0.70 倍以上）で近似する（Python `live_ev.py` 等価）。
///
/// 確率は 2 系統（#272 循環断ち）: `rank_probs`（市場ブレンド・軸/相手の順位付け＝混戦 band も同系列）と
/// `ev_probs`（純モデル・EV/的中の評価）。**両者は同一馬集合でなければならない**（`debug_assert` で検査）。
/// `ev_probs` に順位付け対象の馬が欠けると、その脚の `win` 引きが None になり EV/的中が過小算出される。
/// 現呼び出し側は同一 `entry_factors` 由来の blended/pure を渡すため常に満たす。
pub fn build_portfolio(
    rank_probs: &[HorseProbability],
    ev_probs: &[HorseProbability],
    odds: &RaceOdds,
    race_budget: u64,
    config: &PortfolioConfig,
) -> Portfolio {
    debug_assert!(
        same_field(rank_probs, ev_probs),
        "rank_probs と ev_probs は同一馬集合でなければならない"
    );
    // 軸・相手は rank_probs（市場ブレンド α=0.2）で選ぶ＝解像度の高い本命選定（Phase A: 純モデルは
    // 本命をフラットにしか出せない, #272）。win_prob 降順（同率は馬番昇順）。
    let Some((axis, partners)) = rank_axis_partners(rank_probs, config.partners) else {
        return Portfolio {
            axis: None,
            partners: Vec::new(),
            konsen: false,
            bets: Vec::new(),
            total_stake: 0,
            ev: None,
        };
    };

    // 混戦判定（◎の win_prob 0.70 倍以上が ◎含め 4 頭以上）。band は軸選定と同じ rank_probs で作る
    // （◎=band[0] を保つ）。Python `live_ev.py:is_konsen` と等価。
    let band = band_of(rank_probs);
    let konsen = band.len() >= 4;

    // EV・的中確率は ev_probs（純モデル α=1.0）で評価する＝循環断ち（市場 odds と独立な確率で
    // EV=P_pure×odds を coherent に計算する, #272）。field も ev_probs から（rank と同一馬集合）。
    let win: HashMap<HorseNum, f64> = ev_probs.iter().map(|p| (p.horse_num, p.win_prob)).collect();
    let field: Vec<HorseNum> = ev_probs.iter().map(|p| p.horse_num).collect();

    // 券種ごとの脚（組合せ, odds）を生成する。
    // 連系ペア: 軸-相手の馬連（着順不問）を常に採る。馬単への置換は #262 の 71R バックテストで
    //   純損（順序プレミアムは逆予測）と判明したため #271 で撤去した（ADR 0043 棄却）。
    // ワイド: 軸-相手（保険, 帯の中点）
    let pair_legs: Vec<(BetCombination, Option<f64>)> = partners
        .iter()
        .filter_map(|&p| {
            Pair::try_from((axis, p)).ok().map(|pair| {
                let o = combo_odds(odds, &BetCombination::Quinella(pair));
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
    // 三連複 ◎軸ながし: 軸 + 相手 2 頭（C(K,2) 点）。
    let mut trio: Vec<(BetCombination, Option<f64>)> = Vec::new();
    for i in 0..partners.len() {
        for j in (i + 1)..partners.len() {
            if let Ok(t) = Triple::try_from((axis, partners[i], partners[j])) {
                let o = odds.trio.get(&t).map(|v| v.value());
                trio.push((BetCombination::Trio(t), o));
            }
        }
    }

    // 券種レイヤー `(脚, 相対重み, 方式)`。混戦時のみ CLAUDE.md 配分（`KONSEN_ALLOC`）で印馬3連複
    // ボックスを重ねる（◎が飛んでも印が揃えば拾う保険）。非混戦は現状どおり `config.alloc`（変更なし）。
    // box と ◎軸ながし三連複で同一組番が出うるが、それぞれ別レイヤーとして二重に張る意図（Python 同様）。
    let layers: Vec<BetLayer> = if konsen {
        let (wq, ww, wt, wb) = KONSEN_ALLOC;
        vec![
            (pair_legs, wq, BetMethod::Nagashi),
            (wide, ww, BetMethod::Nagashi),
            (trio, wt, BetMethod::Nagashi),
            (build_box_legs(&band, odds), wb, BetMethod::Box),
        ]
    } else {
        let (wq, ww, wt) = config.alloc;
        vec![
            (pair_legs, wq, BetMethod::Nagashi),
            (wide, ww, BetMethod::Nagashi),
            (trio, wt, BetMethod::Nagashi),
        ]
    };

    // 予算配分（重み→券種予算, 券種内 100 円単位均等配分）。乗算オーバーフロー回避に u128。
    let total_w: u128 = layers.iter().map(|(_, w, _)| *w as u128).sum();
    let mut bets: Vec<PortfolioBet> = Vec::new();
    if let Some(total_w) = std::num::NonZeroU128::new(total_w) {
        for (legs, w, method) in layers {
            // 重み w の券種予算を 100 円単位に floor する（`/100*100`）。
            let type_budget = (race_budget as u128 * w as u128 / total_w.get() / 100 * 100) as u64;
            push_legs(&mut bets, legs, type_budget, &field, &win, method);
        }
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
        konsen,
        bets,
        total_stake,
        ev,
    }
}

/// 1 券種ぶんの脚を予算配分して `out` に積む。`type_budget` を 100 円単位で均等配分し
/// （予算が賄える範囲で薄い脚にも少額が乗る。賄えない端数の脚は ¥0＝買わない）、賭け金 0 の脚は捨てる。
/// 各脚に `method`（ながし / ボックス）を付ける。各脚の ev 倍率・的中確率は `simulate` 単体評価で求める。
///
/// 配分を確率重み＋脚ごと最低¥100 撤廃へ変える案は 71R 検証で実 ROI を悪化させたため不採用
/// （均等割りを維持）。根拠は ADR 0046 / docs/specifications/betting-rule-history.md ⑨。
fn push_legs(
    out: &mut Vec<PortfolioBet>,
    legs: Vec<(BetCombination, Option<f64>)>,
    type_budget: u64,
    field: &[HorseNum],
    win: &HashMap<HorseNum, f64>,
    method: BetMethod,
) {
    let stakes = distribute(type_budget, legs.len());
    for ((combination, odds), stake) in legs.into_iter().zip(stakes) {
        if stake == 0 {
            continue;
        }
        // ev 倍率と的中確率（判断材料として表示）を 1 度の simulate で求める。
        let (ev, hit_prob) = leg_metrics(field, win, &combination, odds);
        out.push(PortfolioBet {
            combination,
            method,
            stake,
            odds,
            ev,
            hit_prob,
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

/// 脚 1 点の (ev 倍率, 的中確率) を求める（的中確率は買い目の判断材料として表示するためのもの。
/// 配分の重みには使わない＝配分は均等割り, ADR 0046）。
///
/// 的中確率（occurrence 確率）は **常にダミー倍率 1.0 の単体 simulate** で求める＝オッズ非依存。
/// 実オッズを simulate に渡すと、`odds=0.0` の汚染値（CLAUDE.md 既知の race_odds 汚染）で payout=0 となり
/// 当たる脚の的中% が 0 に潰れるため、判断ビューでは必ず 1.0 で確率だけを取る。
/// 単一脚の ev 倍率は `的中確率 × floor(100×odds)/100`。simulate の払戻 floor（`payout_of`）に合わせており
/// 従来の `leg_ev`（odds で simulate した roi）と同値になる（テストで担保）。よって simulate は的中確率の
/// 1 度で足り、odds 未取得は精算不能で 0.0。頭数不足（3 頭未満）は (0.0, 0.0)。
fn leg_metrics(
    field: &[HorseNum],
    win: &HashMap<HorseNum, f64>,
    combination: &BetCombination,
    odds: Option<f64>,
) -> (f64, f64) {
    if field.len() < 3 {
        return (0.0, 0.0);
    }
    let hit_prob = simulate(&SimInput {
        field: field.to_vec(),
        bets: vec![PlacedBet {
            combination: combination.clone(),
            stake: 100,
            odds: 1.0,
        }],
        main: None,
        win_probs: Some(win.clone()),
    })
    .ok()
    .and_then(|r| r.ev)
    .map(|e| e.hit_prob)
    .unwrap_or(0.0);
    // 払戻 floor（simulate `payout_of` の floor(stake×odds)）に合わせる＝従来 leg_ev と同値。
    let ev = odds
        .map(|o| hit_prob * (100.0 * o).floor() / 100.0)
        .unwrap_or(0.0);
    (ev, hit_prob)
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
        let pf = build_portfolio(&probs, &probs, &o, 5000, &config);
        assert_eq!(pf.axis, Some(horse(1)));
        assert_eq!(pf.partners, vec![horse(2), horse(3), horse(4)]);
    }

    #[test]
    fn ev_uses_ev_probs_while_ranking_uses_rank_probs() {
        // 循環断ち（#272）の非トートロジー検証: 軸/相手は rank_probs、EV/的中は ev_probs で評価される。
        // rank と ev に**逆向きの win 分布**を与え、両者が別系統で使われることを実証する。
        let (rank, o) = sample(); // 軸=馬1（win 0.40 最大）, 相手=馬2,3,4
        // ev_probs は rank と逆順: 馬5 を最有力・馬1 を最弱にする。EV/的中はこちらに従う。
        let ev = vec![
            prob(1, 0.07),
            prob(2, 0.10),
            prob(3, 0.18),
            prob(4, 0.25),
            prob(5, 0.40),
        ];
        let config = PortfolioConfig {
            partners: 3,
            alloc: (1, 1, 1),
        };
        let pf = build_portfolio(&rank, &ev, &o, 5000, &config);

        // (a) 軸・相手は rank_probs に従う（ev_probs の最有力 馬5 を軸にしない）。
        assert_eq!(pf.axis, Some(horse(1)));
        assert_eq!(pf.partners, vec![horse(2), horse(3), horse(4)]);

        // (b) 各買い目の EV・的中確率は ev_probs の win 分布で算出され、rank_probs では再現しない。
        let ev_win: HashMap<HorseNum, f64> = ev.iter().map(|p| (p.horse_num, p.win_prob)).collect();
        let rank_win: HashMap<HorseNum, f64> =
            rank.iter().map(|p| (p.horse_num, p.win_prob)).collect();
        let field: Vec<HorseNum> = ev.iter().map(|p| p.horse_num).collect();
        let priced: Vec<_> = pf.bets.iter().filter(|b| b.odds.is_some()).collect();
        assert!(!priced.is_empty(), "オッズ取得済みの脚があるはず");
        for b in priced {
            let (exp_ev, exp_hit) = leg_metrics(&field, &ev_win, &b.combination, b.odds);
            assert!((b.ev - exp_ev).abs() < 1e-9, "EV は ev_probs 由来: {b:?}");
            assert!(
                (b.hit_prob - exp_hit).abs() < 1e-9,
                "的中確率は ev_probs 由来: {b:?}"
            );
            // rank_probs で計算すると別値（恒真でないことの担保）。
            let (rank_ev, _) = leg_metrics(&field, &rank_win, &b.combination, b.odds);
            assert!(
                (b.ev - rank_ev).abs() > 1e-9,
                "EV は rank_probs では再現しない（循環断ちの実証）: {b:?}"
            );
        }
    }

    #[test]
    fn generates_axis_nagashi_legs_within_budget_in_100_units() {
        let (probs, o) = sample();
        let config = PortfolioConfig {
            partners: 3,
            alloc: (1, 1, 1),
        };
        let pf = build_portfolio(&probs, &probs, &o, 5000, &config);

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
        let pf = build_portfolio(&probs, &probs, &o, 5000, &config);
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
        let pf = build_portfolio(&probs, &probs, &o, 5000, &config);
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
        let pf = build_portfolio(&probs, &probs, &o, 5000, &PortfolioConfig::default());
        assert!(pf.bets.iter().all(|b| b.stake % 100 == 0 && b.stake > 0));
        assert!(pf.total_stake <= 5000, "stake {} <= 5000", pf.total_stake);
        // 相手 4 頭（馬5まで）→ 馬連4・ワイド4・三連複 C(4,2)=6。
        assert_eq!(pf.partners.len(), 4);
    }

    /// 混戦サンプル: ◎=馬1、0.70×0.30=0.21 以上が 4 頭（馬1..4）→ konsen。馬5(0.05) は band 外。
    /// Python `test_live_ev.py` の konsen fixture {1:30,2:25,3:22,4:21.5,5:5} と同構造。
    fn konsen_sample() -> (Vec<HorseProbability>, RaceOdds) {
        let probs = vec![
            prob(1, 0.30),
            prob(2, 0.25),
            prob(3, 0.22),
            prob(4, 0.215),
            prob(5, 0.05),
        ];
        let mut o = RaceOdds::empty(RaceId::try_from("202506040101".to_string()).unwrap());
        for p in 2..=5 {
            o.quinella
                .insert(Pair::try_from((horse(1), horse(p))).unwrap(), odds(6.0));
            o.wide.insert(
                Pair::try_from((horse(1), horse(p))).unwrap(),
                PlaceOdds::try_from((odds(2.0), odds(3.0))).unwrap(),
            );
        }
        // 三連複は全組合せに odds（◎軸ながし・印馬ボックス双方をカバー）。
        for i in 1..=5 {
            for j in (i + 1)..=5 {
                for k in (j + 1)..=5 {
                    o.trio.insert(
                        Triple::try_from((horse(i), horse(j), horse(k))).unwrap(),
                        odds(30.0),
                    );
                }
            }
        }
        (probs, o)
    }

    #[test]
    fn band_of_returns_horses_within_70pct_in_desc_order() {
        // 混戦: ◎の 0.70 倍以上（0.21 以上）を win_prob 降順で返す＝[1,2,3,4]、馬5 は外れる。
        let (probs, _) = konsen_sample();
        assert_eq!(
            band_of(&probs),
            vec![horse(1), horse(2), horse(3), horse(4)]
        );
        // 非混戦（差が開いたサンプル）: band は 4 頭未満。
        let (flat, _) = sample();
        assert!(band_of(&flat).len() < 4, "band={:?}", band_of(&flat));
    }

    #[test]
    fn konsen_flag_reflects_band_size() {
        let (kprobs, ko) = konsen_sample();
        let kpf = build_portfolio(&kprobs, &kprobs, &ko, 5000, &PortfolioConfig::default());
        assert!(kpf.konsen, "band>=4 で混戦のはず");

        let (fprobs, fo) = sample();
        let fpf = build_portfolio(&fprobs, &fprobs, &fo, 5000, &PortfolioConfig::default());
        assert!(!fpf.konsen, "band<4 は非混戦のはず");
    }

    #[test]
    fn konsen_adds_trio_box_layer_within_budget() {
        let (probs, o) = konsen_sample();
        let pf = build_portfolio(&probs, &probs, &o, 5000, &PortfolioConfig::default());

        // 「◎含め」の要: band[0] が軸（◎）と一致する（rank_probs 同系列で作るため）。
        assert_eq!(band_of(&probs)[0], pf.axis.unwrap(), "band[0] == ◎");

        // 印馬3連複ボックス = band[..5]=[1,2,3,4] の C(4,3)=4 点。全て method=Box・軸なし想定。
        let box_bets: Vec<_> = pf
            .bets
            .iter()
            .filter(|b| b.method == BetMethod::Box)
            .collect();
        assert_eq!(box_bets.len(), 4, "C(4,3)=4 のボックス脚");
        assert!(
            box_bets
                .iter()
                .all(|b| b.combination.type_label() == "trio")
        );

        // ◎軸ながしの三連複も別レイヤーで共存する（method=Nagashi）。
        let nagashi_trio = pf
            .bets
            .iter()
            .filter(|b| b.combination.type_label() == "trio" && b.method == BetMethod::Nagashi)
            .count();
        assert!(nagashi_trio > 0, "◎軸ながし三連複も残る");

        // box と ◎軸ながしで同一組番(1,2,3)が出た場合、dedup せず二重に張る（Python 同様の意図的挙動）。
        let t123: Vec<_> = pf
            .bets
            .iter()
            .filter(|b| b.combination.horse_nums() == vec![1, 2, 3])
            .collect();
        assert_eq!(
            t123.len(),
            2,
            "同一組番(1,2,3)は box と nagashi で 2 本残る"
        );
        assert!(t123.iter().any(|b| b.method == BetMethod::Box));
        assert!(t123.iter().any(|b| b.method == BetMethod::Nagashi));

        // 券種×方式ごとの賭金合計が KONSEN_ALLOC(馬連1000/ワイド1000/3連複ながし1500/box1500) の
        // 配分比を反映する（distribute の floor で各上限以下に収まりつつ、重み比は保たれる）。
        let sum = |bt: &str, m: BetMethod| -> u64 {
            pf.bets
                .iter()
                .filter(|b| b.combination.type_label() == bt && b.method == m)
                .map(|b| b.stake)
                .sum()
        };
        let quinella = sum("quinella", BetMethod::Nagashi);
        let wide = sum("wide", BetMethod::Nagashi);
        let trio_nagashi = sum("trio", BetMethod::Nagashi);
        let box_sum = sum("trio", BetMethod::Box);
        assert!(
            quinella <= 1000 && wide <= 1000 && trio_nagashi <= 1500 && box_sum <= 1500,
            "各レイヤーは KONSEN_ALLOC 上限以下"
        );
        assert_eq!(quinella, wide, "馬連とワイドは同一重み(1000)");
        assert_eq!(
            trio_nagashi, box_sum,
            "3連複ながしとボックスは同一重み(1500)"
        );
        assert!(box_sum > quinella, "1500 重みは 1000 重みより厚い");

        // 予算内・100 円単位。
        assert!(pf.bets.iter().all(|b| b.stake % 100 == 0 && b.stake > 0));
        assert!(pf.total_stake <= 5000, "stake {} <= 5000", pf.total_stake);
    }

    #[test]
    fn konsen_box_caps_at_top5_when_band_exceeds_5() {
        // band が 6 頭以上でも印馬ボックスは上位 5 頭のみ（C(5,3)=10 点）に切り詰める。
        let probs = vec![
            prob(1, 0.20),
            prob(2, 0.19),
            prob(3, 0.18),
            prob(4, 0.17),
            prob(5, 0.16),
            prob(6, 0.15), // 0.70×0.20=0.14 以上 → band 入り（7 頭）
            prob(7, 0.145),
            prob(8, 0.05), // band 外
        ];
        assert_eq!(band_of(&probs).len(), 7, "band は 7 頭");
        let mut o = RaceOdds::empty(RaceId::try_from("202506040101".to_string()).unwrap());
        // 上位 5 頭 [1,2,3,4,5] の三連複ボックス組番に odds を与える。
        for i in 1..=5u32 {
            for j in (i + 1)..=5 {
                for k in (j + 1)..=5 {
                    o.trio.insert(
                        Triple::try_from((horse(i), horse(j), horse(k))).unwrap(),
                        odds(30.0),
                    );
                }
            }
        }
        let pf = build_portfolio(&probs, &probs, &o, 5000, &PortfolioConfig::default());
        assert!(pf.konsen);
        let box_count = pf
            .bets
            .iter()
            .filter(|b| b.method == BetMethod::Box)
            .count();
        assert_eq!(box_count, 10, "band 上位 5 頭 C(5,3)=10 に切り詰め");
    }

    #[test]
    fn band_of_includes_boundary_and_breaks_ties_by_horse_num() {
        // 同率は馬番昇順・閾値近傍の採否を検証（Python band_of と等価: -prob ソート + 挿入=馬番順）。
        // 厳密境界（win == 0.70×max）の等価比較は Rust/Python とも float 誤差でどちらにも転びうるため、
        // ここではマージンを持たせた採否（0.145>閾値0.14・0.13<0.14）と同率のタイブレークを固める。
        let probs = vec![
            prob(1, 0.20),
            prob(2, 0.20),  // 1 と同率 → 馬番昇順で 1,2
            prob(3, 0.15),  // 閾値 0.14 超 → 採用
            prob(4, 0.145), // 閾値 0.14 超 → 採用
            prob(5, 0.13),  // 閾値未満 → 除外
        ];
        assert_eq!(
            band_of(&probs),
            vec![horse(1), horse(2), horse(3), horse(4)],
            "同率は馬番昇順・0.13 は band 外"
        );
    }

    #[test]
    fn non_konsen_has_no_box_layer() {
        let (probs, o) = sample();
        let pf = build_portfolio(&probs, &probs, &o, 5000, &PortfolioConfig::default());
        assert!(!pf.konsen);
        assert!(
            pf.bets.iter().all(|b| b.method == BetMethod::Nagashi),
            "非混戦はボックスを組まない"
        );
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
        let pf = build_portfolio(&probs, &probs, &o, 5000, &config);
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
        let pf = build_portfolio(&[], &[], &o, 5000, &PortfolioConfig::default());
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
        let pf = build_portfolio(&probs, &probs, &o, 200, &config);
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
    fn pair_legs_are_always_quinella_even_with_high_exacta() {
        // #271: 馬連→馬単の EV 置換は #262 の 71R バックテストで純損（順序プレミアムは逆予測）と
        // 判明したため撤去した。高オッズの馬単を与えても連系脚は一切 Exacta にならず全て Quinella。
        let (probs, mut o) = sample();
        o.exacta.insert(
            OrderedPair::try_from((horse(1), horse(2))).unwrap(),
            odds(100.0),
        );
        let config = PortfolioConfig {
            partners: 3,
            alloc: (1, 0, 0), // 連系ペアのみに絞って検証
        };
        let pf = build_portfolio(&probs, &probs, &o, 5000, &config);
        assert!(
            !pf.bets
                .iter()
                .any(|b| matches!(b.combination, BetCombination::Exacta(_))),
            "馬単置換は撤去済み（常に馬連）: {:?}",
            pf.bets
        );
        // 連系ペアは相手 3 頭ぶん全て馬連。
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
        let diag = pair_ev_diagnostics(&probs, &probs, &o, 3);
        assert_eq!(diag.axis, Some(horse(1)));
        let rows = &diag.rows;
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
        let diag = pair_ev_diagnostics(&[], &[], &o, 5);
        assert_eq!(diag.axis, None);
        assert!(diag.rows.is_empty());
    }

    #[test]
    fn portfolio_preserves_budget_and_units() {
        // 既定配分でも 100 円単位・予算内が保たれる（連系ペアは常に馬連）。
        let (probs, o) = sample();
        let pf = build_portfolio(&probs, &probs, &o, 5000, &PortfolioConfig::default());
        assert!(pf.bets.iter().all(|b| b.stake % 100 == 0 && b.stake > 0));
        assert!(pf.total_stake <= 5000, "stake {} <= 5000", pf.total_stake);
    }

    #[test]
    fn pair_leg_kept_as_quinella_when_quinella_odds_missing() {
        // 馬連オッズ欠落でも連系脚は Quinella（odds=None）として生成され、ペアが脱落しない。
        // 撤去した pick_pair_leg のオッズ欠落ハンドリングが担保していた不変条件を維持する。
        let (probs, mut o) = sample();
        o.quinella
            .remove(&Pair::try_from((horse(1), horse(2))).unwrap());
        let config = PortfolioConfig {
            partners: 3,
            alloc: (1, 0, 0),
        };
        let pf = build_portfolio(&probs, &probs, &o, 5000, &config);
        let leg = pf.bets.iter().find(|b| {
            matches!(&b.combination, BetCombination::Quinella(p)
                if p.as_tuple() == (horse(1), horse(2)))
        });
        assert!(
            leg.is_some(),
            "馬連オッズ欠落でも Quinella 脚は生成される: {:?}",
            pf.bets
        );
        assert_eq!(leg.unwrap().odds, None, "欠落オッズは None のまま");
        assert!(
            !pf.bets
                .iter()
                .any(|b| matches!(b.combination, BetCombination::Exacta(_))),
            "Exacta 化はしない: {:?}",
            pf.bets
        );
    }

    #[test]
    fn portfolio_populates_hit_prob_for_each_bet() {
        // 各買い目に的中確率（判断材料）が入る。確率なので (0, 1] の範囲。
        // sample() の相手は全て勝率 > 0 なので hit_prob > 0 が保証される（勝率 0 の相手なら 0.0 になりうる）。
        let (probs, o) = sample();
        let pf = build_portfolio(&probs, &probs, &o, 5000, &PortfolioConfig::default());
        assert!(!pf.bets.is_empty());
        assert!(
            pf.bets
                .iter()
                .all(|b| b.hit_prob > 0.0 && b.hit_prob <= 1.0),
            "全買い目に的中確率(0,1]が入る: {:?}",
            pf.bets
        );
        // ev 倍率は従来の leg_ev（simulate 由来・払戻 floor 込み）と一致する（恒真でない独立検証）。
        let win: HashMap<HorseNum, f64> = probs.iter().map(|p| (p.horse_num, p.win_prob)).collect();
        let field: Vec<HorseNum> = probs.iter().map(|p| p.horse_num).collect();
        for b in pf.bets.iter().filter(|b| b.odds.is_some()) {
            let expected = leg_ev(&field, &win, &b.combination, b.odds);
            assert!(
                (b.ev - expected).abs() < 1e-9,
                "ev {} == leg_ev {}: {:?}",
                b.ev,
                expected,
                b
            );
        }
    }

    #[test]
    fn hit_prob_is_computed_even_when_odds_missing() {
        // 的中確率はオッズ非依存。三連複オッズを空にしても hit_prob は算出される（ev のみ 0）。
        let (probs, mut o) = sample();
        o.trio.clear();
        let pf = build_portfolio(&probs, &probs, &o, 5000, &PortfolioConfig::default());
        let trio: Vec<_> = pf
            .bets
            .iter()
            .filter(|b| matches!(b.combination, BetCombination::Trio(_)))
            .collect();
        assert!(!trio.is_empty());
        assert!(
            trio.iter()
                .all(|b| b.odds.is_none() && b.ev == 0.0 && b.hit_prob > 0.0),
            "オッズ欠落でも的中確率は出る（ev だけ 0）: {:?}",
            trio
        );
    }
}
