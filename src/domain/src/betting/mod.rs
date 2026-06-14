use std::collections::HashMap;

use ordered_float::OrderedFloat;

use crate::horse_result::HorseNum;
use crate::odds::{BetType, OrderedPair, OrderedTriple, Pair, RaceOdds, Triple};
use crate::prediction::HorseProbability;

const MIN_DENOMINATOR: f64 = 1e-6;

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

/// Returns EV-positive bet recommendations sorted by bet-type priority then EV descending.
///
/// Priority (sort key): Quinella(0) > Exacta(1) > Trio(2) > Win(3) > Place(4) > Trifecta(5).
/// Smaller sort key = earlier in the result. Trifecta requires `ev > trifecta_ev_threshold`;
/// all other bet types require `ev > ev_threshold` (strict greater-than; ev = threshold is excluded).
/// When two recommendations share the same priority and EV, they are ordered by horse numbers
/// for deterministic output.
///
/// Note: `Place` (複勝) uses `HorseProbability::show_prob` (3着以内確率) as the probability estimate.
/// `place_prob` (2着以内確率) is not used for any bet type in this function.
pub fn select_bets(
    probabilities: &[HorseProbability],
    race_odds: &RaceOdds,
    config: &BettingConfig,
) -> Vec<BettingRecommendation> {
    let prob_map: HashMap<HorseNum, &HorseProbability> =
        probabilities.iter().map(|p| (p.horse_num, p)).collect();

    let mut recs: Vec<BettingRecommendation> = Vec::new();

    for (&horse, &ov) in &race_odds.win {
        if let Some(hp) = prob_map.get(&horse) {
            let o = ov.value();
            push_if_positive(
                &mut recs,
                BetCombination::Win(horse),
                hp.win_prob,
                o,
                config.ev_threshold,
                config,
            );
        }
    }

    for (&horse, place_ov) in &race_odds.place {
        if let Some(hp) = prob_map.get(&horse) {
            // JRA 複勝は「3 着以内入線」に相当するため show_prob（3 着以内確率）を使用する。
            // place_prob は「2 着以内確率」（連対率）であり、複勝計算では使わない。
            // 未確定幅 (low..high) の中央値を期待値計算の代表値とする。
            let o = (place_ov.low.value() + place_ov.high.value()) / 2.0;
            push_if_positive(
                &mut recs,
                BetCombination::Place(horse),
                hp.show_prob,
                o,
                config.ev_threshold,
                config,
            );
        }
    }

    for (&pair, &ov) in &race_odds.quinella {
        let (a, b) = pair.as_tuple();
        if let (Some(ha), Some(hb)) = (prob_map.get(&a), prob_map.get(&b)) {
            let p = harville_quinella(ha.win_prob, hb.win_prob);
            let o = ov.value();
            push_if_positive(
                &mut recs,
                BetCombination::Quinella(pair),
                p,
                o,
                config.ev_threshold,
                config,
            );
        }
    }

    for (&pair, &ov) in &race_odds.exacta {
        let (a, b) = pair.as_tuple();
        if let (Some(ha), Some(hb)) = (prob_map.get(&a), prob_map.get(&b)) {
            let p = harville_exacta(ha.win_prob, hb.win_prob);
            let o = ov.value();
            push_if_positive(
                &mut recs,
                BetCombination::Exacta(pair),
                p,
                o,
                config.ev_threshold,
                config,
            );
        }
    }

    for (&triple, &ov) in &race_odds.trio {
        let (a, b, c) = triple.as_tuple();
        if let (Some(ha), Some(hb), Some(hc)) =
            (prob_map.get(&a), prob_map.get(&b), prob_map.get(&c))
        {
            let p = harville_trio(ha.win_prob, hb.win_prob, hc.win_prob);
            let o = ov.value();
            push_if_positive(
                &mut recs,
                BetCombination::Trio(triple),
                p,
                o,
                config.ev_threshold,
                config,
            );
        }
    }

    for (&triple, &ov) in &race_odds.trifecta {
        let (a, b, c) = triple.as_tuple();
        if let (Some(ha), Some(hb), Some(hc)) =
            (prob_map.get(&a), prob_map.get(&b), prob_map.get(&c))
        {
            let p = harville_trifecta(ha.win_prob, hb.win_prob, hc.win_prob);
            let o = ov.value();
            push_if_positive(
                &mut recs,
                BetCombination::Trifecta(triple),
                p,
                o,
                config.trifecta_ev_threshold,
                config,
            );
        }
    }

    recs.sort_by_key(|r| {
        (
            priority(&r.combination),
            OrderedFloat(-r.ev),
            combination_ord_key(&r.combination),
        )
    });

    // curation（#121）: 全正EVダンプを止める。EV 閾値通過後の買い目から、
    // (1) Kelly が薄い（過信に支えられた一点¥1〜3 級の）買い目を落とし、
    // (2) 券種ごとに EV 上位 N 点へ絞る。recs は priority→EV 降順済みで、券種は
    //     priority と 1:1 のため、type 別カウンタで残せば EV 上位 N が保たれる。
    recs.retain(|r| r.kelly_fraction > config.min_kelly);
    if let Some(n) = config.max_bets_per_type {
        let mut per_type: HashMap<&'static str, usize> = HashMap::new();
        recs.retain(|r| {
            let c = per_type.entry(r.combination.type_label()).or_insert(0);
            if *c < n {
                *c += 1;
                true
            } else {
                false
            }
        });
    }
    recs
}

fn combination_ord_key(c: &BetCombination) -> (u32, u32, u32) {
    match c {
        BetCombination::Win(h) | BetCombination::Place(h) => (h.value(), 0, 0),
        BetCombination::Quinella(p) | BetCombination::Wide(p) => {
            let (a, b) = p.as_tuple();
            (a.value(), b.value(), 0)
        }
        BetCombination::Exacta(p) => {
            let (a, b) = p.as_tuple();
            (a.value(), b.value(), 0)
        }
        BetCombination::Trio(t) => {
            let (a, b, c) = t.as_tuple();
            (a.value(), b.value(), c.value())
        }
        BetCombination::Trifecta(t) => {
            let (a, b, c) = t.as_tuple();
            (a.value(), b.value(), c.value())
        }
    }
}

fn push_if_positive(
    recs: &mut Vec<BettingRecommendation>,
    combination: BetCombination,
    probability: f64,
    odds: f64,
    ev_threshold: f64,
    config: &BettingConfig,
) {
    let ev = probability * odds;
    if ev > ev_threshold {
        recs.push(BettingRecommendation {
            combination,
            probability,
            odds,
            ev,
            kelly_fraction: kelly_fraction(probability, odds, config.kelly_cap),
        });
    }
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
    fn in_the_money(&self, h: HorseNum) -> bool {
        let target = Some(h);
        if self.first == target || self.second == target {
            return true;
        }
        // 3 着が払戻圏に入るのは 8 頭以上のときだけ。
        self.field_size >= 8 && self.third == target
    }
}

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

fn priority(c: &BetCombination) -> u8 {
    match c {
        BetCombination::Quinella(_) => 0,
        BetCombination::Exacta(_) => 1,
        BetCombination::Trio(_) => 2,
        BetCombination::Win(_) => 3,
        BetCombination::Place(_) => 4,
        BetCombination::Trifecta(_) => 5,
        // Wide は select_bets では生成しない（収支シミュレータ専用）。網羅性のため末尾に置く。
        BetCombination::Wide(_) => 6,
    }
}

/// P(a→b): Harville conditional probability that b finishes 2nd given a wins.
///
/// Returns `0.0` when `win_a >= 1.0` (denominator `1 - win_a` would be zero or negative).
/// Unlike `harville_trifecta`, the guard here only checks `win_a` because `win_b`
/// does not appear in the denominator.
pub(crate) fn harville_exacta(win_a: f64, win_b: f64) -> f64 {
    if win_a >= 1.0 {
        return 0.0;
    }
    let denom = (1.0 - win_a).max(MIN_DENOMINATOR);
    win_a * win_b / denom
}

/// P(quinella {a,b}) = P(a→b) + P(b→a).
pub(crate) fn harville_quinella(win_a: f64, win_b: f64) -> f64 {
    harville_exacta(win_a, win_b) + harville_exacta(win_b, win_a)
}

/// P(trifecta a→b→c): Harville sequential conditional probability.
///
/// Precondition: `win_a + win_b < 1.0`. Returns `0.0` when this is violated
/// to avoid a negative denominator being clamped to MIN_DENOMINATOR, which
/// would produce an unrealistically large probability.
pub(crate) fn harville_trifecta(win_a: f64, win_b: f64, win_c: f64) -> f64 {
    if win_a + win_b >= 1.0 {
        return 0.0;
    }
    let denom_a = (1.0 - win_a).max(MIN_DENOMINATOR);
    // The guard ensures 1-win_a-win_b > 0, but min-clamp is kept for floating-point safety
    // when win_a+win_b is very close to 1.0.
    let denom_ab = (1.0 - win_a - win_b).max(MIN_DENOMINATOR);
    win_a * (win_b / denom_a) * (win_c / denom_ab)
}

/// P(trio {a,b,c}) = sum of all 6 ordered permutations as trifecta probabilities.
pub(crate) fn harville_trio(win_a: f64, win_b: f64, win_c: f64) -> f64 {
    harville_trifecta(win_a, win_b, win_c)
        + harville_trifecta(win_a, win_c, win_b)
        + harville_trifecta(win_b, win_a, win_c)
        + harville_trifecta(win_b, win_c, win_a)
        + harville_trifecta(win_c, win_a, win_b)
        + harville_trifecta(win_c, win_b, win_a)
}

/// Kelly fraction with cap: f = (p*b - q) / b, clamped to [0, kelly_cap].
///
/// `gross_odds` is the JRA payout multiplier (e.g. 3.5 means ¥350 back on ¥100).
/// Net odds b = gross_odds - 1.0 (gross → net 変換). EV = p * gross_odds; EV > 1.0 が期待値プラス。
/// Returns `0.0` when `gross_odds <= 1.0` (no net profit possible, avoids zero division).
pub(crate) fn kelly_fraction(p: f64, gross_odds: f64, kelly_cap: f64) -> f64 {
    let b = gross_odds - 1.0;
    if b <= 0.0 {
        return 0.0;
    }
    let q = 1.0 - p;
    let f = (p * b - q) / b;
    f.max(0.0).min(kelly_cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horse_result::HorseNum;
    use crate::odds::{OddsValue, PlaceOdds, RaceOdds};
    use crate::prediction::HorseProbability;
    use crate::race::RaceId;

    fn make_race_id() -> RaceId {
        RaceId::try_from("202506040101".to_string()).unwrap()
    }

    fn horse(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    fn odds(v: f64) -> OddsValue {
        OddsValue::try_from(v).unwrap()
    }

    fn place_odds(lo: f64, hi: f64) -> PlaceOdds {
        PlaceOdds::try_from((odds(lo), odds(hi))).unwrap()
    }

    fn prob(horse_num: u32, win: f64, show: f64) -> HorseProbability {
        use crate::horse_result::HorseName;
        HorseProbability {
            horse_num: horse(horse_num),
            horse_name: HorseName::try_from(format!("ウマ{horse_num}")).unwrap(),
            win_prob: win,
            // place_prob（2着以内確率）は select_bets で使わない。
            // 0.0 にしておくことで誤って参照された場合に EV が0になりすぐ気づける。
            place_prob: 0.0,
            show_prob: show,
        }
    }

    #[test]
    fn empty_probabilities_returns_empty() {
        let race_odds = RaceOdds::empty(make_race_id());
        let result = select_bets(&[], &race_odds, &BettingConfig::default());
        assert!(result.is_empty());
    }

    #[test]
    fn empty_odds_returns_empty() {
        let probs = vec![prob(1, 0.5, 0.7), prob(2, 0.3, 0.5)];
        let race_odds = RaceOdds::empty(make_race_id());
        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert!(result.is_empty());
    }

    #[test]
    fn win_bet_above_threshold_is_included() {
        let probs = vec![prob(1, 0.4, 0.6)];
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.win.insert(horse(1), odds(3.5));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert_eq!(result.len(), 1);
        let r = &result[0];
        assert!((r.ev - 0.4 * 3.5).abs() < 1e-10);
        assert!(r.ev > 1.0);
    }

    #[test]
    fn win_bet_below_threshold_is_excluded() {
        // p=0.2, odds=4.0, ev=0.8 < 1.0
        let probs = vec![prob(1, 0.2, 0.4)];
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.win.insert(horse(1), odds(4.0));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert!(result.is_empty());
    }

    #[test]
    fn trifecta_uses_higher_threshold() {
        // harville_trifecta(0.4, 0.35, 0.2) ≈ 0.4 * 0.35/0.6 * 0.2/0.25 ≈ 0.187
        // EV ≈ 0.187 * 8.0 ≈ 1.5 → above ev_threshold(1.0) but below trifecta_ev_threshold(2.0)
        let probs = vec![prob(1, 0.4, 0.6), prob(2, 0.35, 0.55), prob(3, 0.2, 0.4)];
        let (a, b, c) = (horse(1), horse(2), horse(3));
        let triple = OrderedTriple::try_from((a, b, c)).unwrap();
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.trifecta.insert(triple, odds(8.0));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert!(
            result.is_empty(),
            "trifecta with EV < 2.0 should be excluded"
        );
    }

    #[test]
    fn trifecta_above_trifecta_threshold_is_included() {
        let probs = vec![prob(1, 0.4, 0.6), prob(2, 0.35, 0.55), prob(3, 0.2, 0.4)];
        let (a, b, c) = (horse(1), horse(2), horse(3));
        let triple = OrderedTriple::try_from((a, b, c)).unwrap();
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.trifecta.insert(triple, odds(20.0));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert!(!result.is_empty());
        let r = &result[0];
        assert!(r.ev > 2.0);
    }

    #[test]
    fn kelly_fraction_is_capped() {
        let probs = vec![prob(1, 0.9, 0.95)];
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.win.insert(horse(1), odds(2.0));

        let config = BettingConfig {
            ev_threshold: 1.0,
            trifecta_ev_threshold: 2.0,
            kelly_cap: 0.25,
            min_kelly: 0.0,
            max_bets_per_type: None,
        };
        let result = select_bets(&probs, &race_odds, &config);
        assert!(!result.is_empty());
        assert!(result[0].kelly_fraction <= 0.25);
    }

    #[test]
    fn quinella_priority_before_win() {
        let probs = vec![prob(1, 0.5, 0.7), prob(2, 0.3, 0.55)];
        let pair = Pair::try_from((horse(1), horse(2))).unwrap();
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.win.insert(horse(1), odds(2.5)); // EV = 0.5*2.5 = 1.25
        race_odds.quinella.insert(pair, odds(5.0));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert!(
            result
                .iter()
                .any(|r| matches!(r.combination, BetCombination::Quinella(_)))
        );
        assert!(
            result
                .iter()
                .any(|r| matches!(r.combination, BetCombination::Win(_)))
        );
        assert!(matches!(result[0].combination, BetCombination::Quinella(_)));
    }

    #[test]
    fn place_uses_midpoint_odds() {
        let probs = vec![prob(1, 0.4, 0.6)];
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.place.insert(horse(1), place_odds(2.0, 4.0));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        // midpoint = (2.0+4.0)/2 = 3.0, EV = 0.6*3.0 = 1.8 > 1.0
        assert_eq!(result.len(), 1);
        assert!((result[0].odds - 3.0).abs() < 1e-10);
        assert!((result[0].ev - 1.8).abs() < 1e-10);
    }

    #[test]
    fn min_kelly_filters_thin_positive_ev_bets() {
        // EV はわずかに正だが Kelly が薄い買い目（全正EVダンプの主因, #121）。
        // p=0.05, odds=21 → EV=1.05>1.0 だが Kelly=(0.05*20-0.95)/20=0.0025。
        let probs = vec![prob(1, 0.05, 0.1)];
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.win.insert(horse(1), odds(21.0));

        // uncurated（min_kelly=0）なら EV>1 で採用される。
        let uncurated = BettingConfig {
            ev_threshold: 1.0,
            trifecta_ev_threshold: 2.0,
            kelly_cap: 0.25,
            min_kelly: 0.0,
            max_bets_per_type: None,
        };
        assert_eq!(select_bets(&probs, &race_odds, &uncurated).len(), 1);
        // 既定（min_kelly=0.01）では Kelly 薄として除外される。
        assert!(select_bets(&probs, &race_odds, &BettingConfig::default()).is_empty());
    }

    #[test]
    fn max_bets_per_type_caps_to_top_n_by_ev() {
        // 同一券種（単勝）で +EV かつ Kelly 十分な 4 頭を用意し、券種上限 2 に絞る。
        let probs = vec![
            prob(1, 0.5, 0.7),
            prob(2, 0.45, 0.65),
            prob(3, 0.4, 0.6),
            prob(4, 0.35, 0.55),
        ];
        let mut race_odds = RaceOdds::empty(make_race_id());
        for h in 1..=4 {
            race_odds.win.insert(horse(h), odds(3.0)); // EV = win_prob * 3.0
        }
        let config = BettingConfig {
            ev_threshold: 1.0,
            trifecta_ev_threshold: 2.0,
            kelly_cap: 0.25,
            min_kelly: 0.0,
            max_bets_per_type: Some(2),
        };
        let result = select_bets(&probs, &race_odds, &config);
        let wins: Vec<_> = result
            .iter()
            .filter(|r| matches!(r.combination, BetCombination::Win(_)))
            .collect();
        assert_eq!(wins.len(), 2, "券種上限 2 に絞られる");
        // EV 上位（馬1=1.5, 馬2=1.35）が残り、下位（馬3=1.2, 馬4=1.05）は落ちる。
        assert!(wins.iter().all(|r| r.ev >= 1.35 - 1e-9));
    }

    #[test]
    fn min_kelly_and_max_bets_per_type_compose() {
        // 両 curation レバーの合成。単勝 5 頭: 上位 4 頭は Kelly 十分、5 頭目は EV>1 だが Kelly 薄。
        // min_kelly が先に 5 頭目を落とし、その後 max_bets_per_type=2 で EV 上位 2 点に絞られる。
        let probs = vec![
            prob(1, 0.5, 0.7),
            prob(2, 0.45, 0.65),
            prob(3, 0.4, 0.6),
            prob(4, 0.35, 0.55),
            prob(5, 0.05, 0.1), // EV=1.05 だが Kelly=0.0025（薄い）
        ];
        let mut race_odds = RaceOdds::empty(make_race_id());
        for h in 1..=4 {
            race_odds.win.insert(horse(h), odds(3.0)); // EV = win_prob * 3.0
        }
        race_odds.win.insert(horse(5), odds(21.0)); // EV=1.05

        let config = BettingConfig {
            ev_threshold: 1.0,
            trifecta_ev_threshold: 2.0,
            kelly_cap: 0.25,
            min_kelly: 0.01,
            max_bets_per_type: Some(2),
        };
        let result = select_bets(&probs, &race_odds, &config);
        assert_eq!(result.len(), 2, "min_kelly 除外後に券種上限 2 で絞られる");
        // 残るのは EV 上位の馬1(1.5)・馬2(1.35)。Kelly 薄の馬5 は min_kelly で先に脱落。
        assert!(result.iter().all(|r| r.ev >= 1.35 - 1e-9));
        assert!(
            result
                .iter()
                .all(|r| !matches!(&r.combination, BetCombination::Win(h) if h.value() == 5)),
            "Kelly 薄の馬5 は除外される"
        );
    }

    #[test]
    fn bet_hit_judges_each_bet_type() {
        // 確定: 1着=3, 2着=1, 3着=5。8 頭立て（複勝/ワイドは 3 着以内まで払戻圏）。
        let podium = Podium {
            first: Some(horse(3)),
            second: Some(horse(1)),
            third: Some(horse(5)),
            field_size: 8,
        };
        assert!(bet_hit(&BetCombination::Win(horse(3)), &podium));
        assert!(!bet_hit(&BetCombination::Win(horse(1)), &podium));
        assert!(bet_hit(&BetCombination::Place(horse(5)), &podium));
        assert!(!bet_hit(&BetCombination::Place(horse(2)), &podium));
        let q = |a, b| BetCombination::Quinella(Pair::try_from((horse(a), horse(b))).unwrap());
        assert!(bet_hit(&q(1, 3), &podium)); // {1,2着}={3,1}
        assert!(!bet_hit(&q(3, 5), &podium));
        let ex =
            |a, b| BetCombination::Exacta(OrderedPair::try_from((horse(a), horse(b))).unwrap());
        assert!(bet_hit(&ex(3, 1), &podium)); // 1→2着=3→1
        assert!(!bet_hit(&ex(1, 3), &podium));
        let wd = |a, b| BetCombination::Wide(Pair::try_from((horse(a), horse(b))).unwrap());
        assert!(bet_hit(&wd(1, 5), &podium)); // 両馬3着以内
        assert!(!bet_hit(&wd(1, 2), &podium));
        let tr = |a, b, c| {
            BetCombination::Trio(Triple::try_from((horse(a), horse(b), horse(c))).unwrap())
        };
        assert!(bet_hit(&tr(5, 3, 1), &podium)); // {1,2,3着}無順
        assert!(!bet_hit(&tr(1, 2, 3), &podium));
        let tf = |a, b, c| {
            BetCombination::Trifecta(
                OrderedTriple::try_from((horse(a), horse(b), horse(c))).unwrap(),
            )
        };
        assert!(bet_hit(&tf(3, 1, 5), &podium)); // 1→2→3着
        assert!(!bet_hit(&tf(3, 5, 1), &podium));
    }

    #[test]
    fn bet_hit_false_when_podium_incomplete() {
        // 1 着しか確定していない（同着・着順欠落など）。
        let podium = Podium {
            first: Some(horse(3)),
            second: None,
            third: None,
            field_size: 8,
        };
        assert!(bet_hit(&BetCombination::Win(horse(3)), &podium)); // 単勝は 1 着のみで判定可
        let q = BetCombination::Quinella(Pair::try_from((horse(3), horse(1))).unwrap());
        assert!(!bet_hit(&q, &podium)); // 2 着未確定 → 非的中
        let tf = BetCombination::Trifecta(
            OrderedTriple::try_from((horse(3), horse(1), horse(5))).unwrap(),
        );
        assert!(!bet_hit(&tf, &podium));
    }

    #[test]
    fn bet_hit_place_and_wide_depend_on_field_size() {
        // 確定: 1着=3, 2着=1, 3着=5。複勝/ワイドの払戻圏は頭数依存（JRA: 8頭以上=3着, 7頭以下=2着）。
        let make = |field_size| Podium {
            first: Some(horse(3)),
            second: Some(horse(1)),
            third: Some(horse(5)),
            field_size,
        };
        // 7 頭立て: 3 着(=5)は払戻圏外。複勝 5 は不的中、ワイド{1,5}も 5 が圏外で不的中。
        let small = make(7);
        assert!(bet_hit(&BetCombination::Place(horse(1)), &small)); // 2 着は払戻圏
        assert!(!bet_hit(&BetCombination::Place(horse(5)), &small)); // 3 着は圏外
        let wd = |a, b| BetCombination::Wide(Pair::try_from((horse(a), horse(b))).unwrap());
        assert!(!bet_hit(&wd(1, 5), &small)); // 5 が圏外
        assert!(bet_hit(&wd(3, 1), &small)); // 1着・2着で両方圏内

        // 8 頭立て: 3 着(=5)まで払戻圏。複勝 5・ワイド{1,5}とも的中。
        let large = make(8);
        assert!(bet_hit(&BetCombination::Place(horse(5)), &large));
        assert!(bet_hit(&wd(1, 5), &large));
    }

    #[test]
    fn harville_exacta_formula() {
        let result = harville_exacta(0.4, 0.3);
        let expected = 0.4 * 0.3 / (1.0 - 0.4);
        assert!((result - expected).abs() < 1e-10);
    }

    #[test]
    fn harville_exacta_returns_zero_when_first_horse_wins_with_certainty() {
        // win_a >= 1.0 → denominator (1-win_a) <= 0 → guard returns 0.0
        assert_eq!(harville_exacta(1.0, 0.3), 0.0);
    }

    #[test]
    fn harville_exacta_valid_when_only_win_a_is_below_one() {
        // win_a + win_b can exceed 1.0 as long as win_a < 1.0 (denominator is positive)
        let result = harville_exacta(0.6, 0.5);
        assert!(result > 0.0, "expected positive probability, got {result}");
    }

    #[test]
    fn harville_quinella_is_sum_of_two_exactas() {
        let wa = 0.4;
        let wb = 0.3;
        let q = harville_quinella(wa, wb);
        let expected = harville_exacta(wa, wb) + harville_exacta(wb, wa);
        assert!((q - expected).abs() < 1e-10);
    }

    #[test]
    fn harville_trifecta_returns_zero_when_first_two_exhaust_probability() {
        // win_a + win_b >= 1.0 → guard returns 0.0 instead of clamped huge value
        assert_eq!(harville_trifecta(0.6, 0.5, 0.1), 0.0);
        assert_eq!(harville_trifecta(1.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn harville_trio_with_near_unit_sum_returns_finite_value() {
        // wa+wb+wc = 0.95; some permutations will trigger trifecta guard
        // (e.g. wb+wa = 0.5+0.4 = 0.9 < 1.0 is ok; but wc=0.05 combos are fine)
        let trio = harville_trio(0.5, 0.4, 0.05);
        assert!(trio >= 0.0);
        assert!(
            trio <= 1.0,
            "trio probability should not exceed 1.0, got {trio}"
        );
    }

    #[test]
    fn harville_trio_is_sum_of_six_trifectas() {
        let (wa, wb, wc) = (0.4, 0.3, 0.2);
        let trio = harville_trio(wa, wb, wc);
        let expected = harville_trifecta(wa, wb, wc)
            + harville_trifecta(wa, wc, wb)
            + harville_trifecta(wb, wa, wc)
            + harville_trifecta(wb, wc, wa)
            + harville_trifecta(wc, wa, wb)
            + harville_trifecta(wc, wb, wa);
        assert!((trio - expected).abs() < 1e-10);
    }

    #[test]
    fn kelly_fraction_basic() {
        // p=0.4, odds=3.5: b=2.5, q=0.6, f=(0.4*2.5-0.6)/2.5=0.16
        let kf = kelly_fraction(0.4, 3.5, 0.25);
        assert!((kf - 0.16).abs() < 1e-10);
    }

    #[test]
    fn kelly_fraction_negative_clamped_to_zero() {
        // p=0.1, odds=2.0: b=1.0, q=0.9, f=(0.1-0.9)/1.0=-0.8 → clamped to 0
        let kf = kelly_fraction(0.1, 2.0, 0.25);
        assert_eq!(kf, 0.0);
    }

    #[test]
    fn kelly_fraction_respects_cap() {
        let kf = kelly_fraction(0.95, 2.0, 0.25);
        assert_eq!(kf, 0.25);
    }

    #[test]
    fn bet_combination_encodes_type_and_code() {
        let win = BetCombination::Win(horse(3));
        assert_eq!(win.type_label(), "win");
        assert_eq!(win.combination_code(), "3");

        let place = BetCombination::Place(horse(7));
        assert_eq!(place.type_label(), "place");
        assert_eq!(place.combination_code(), "7");

        let quinella = BetCombination::Quinella(Pair::try_from((horse(5), horse(1))).unwrap());
        assert_eq!(quinella.type_label(), "quinella");
        // Pair は昇順に正規化されるため "1-5"
        assert_eq!(quinella.combination_code(), "1-5");

        let exacta = BetCombination::Exacta(OrderedPair::try_from((horse(1), horse(5))).unwrap());
        assert_eq!(exacta.type_label(), "exacta");
        assert_eq!(exacta.combination_code(), "1>5");

        let trio = BetCombination::Trio(Triple::try_from((horse(3), horse(1), horse(5))).unwrap());
        assert_eq!(trio.type_label(), "trio");
        // Triple は昇順に正規化されるため "1-3-5"
        assert_eq!(trio.combination_code(), "1-3-5");

        let trifecta = BetCombination::Trifecta(
            OrderedTriple::try_from((horse(1), horse(3), horse(5))).unwrap(),
        );
        assert_eq!(trifecta.type_label(), "trifecta");
        assert_eq!(trifecta.combination_code(), "1>3>5");
    }
}
