//! Portfolio（ドメイン）→ `LiveEvSnapshotRecord`（use-case DTO）への純粋写像（#346 / ADR 0064）。
//!
//! predict-watch が 1 スイープ 1 レースを評価した結果（買い目 Portfolio ＋ ◎確率・オッズ）を、
//! `live_ev_snapshots` へ書ける 1 レコードへ落とす。DB・時計に触れない純関数なのでユニットテスト可能。
//! 混戦（box）は MVP では組まないため `konsen=false`・全 leg が `nagashi`（#352 で移植）。

use chrono::NaiveDate;
use paddock_domain::Portfolio;
use paddock_use_case::repository::{LiveEvSnapshotRecord, SlipLegRecord};

/// Portfolio 以外に写像へ要るレース単位のスカラ文脈（確率・オッズ・識別子）。
pub struct SnapshotContext<'a> {
    /// 開催日（`live_ev_snapshots.date`）。
    pub date: NaiveDate,
    pub race_id: &'a str,
    /// 開催場 slug（例 `"tokyo"`。SPA が JP へ写像するため slug で持つ）。
    pub venue: &'a str,
    pub race_no: u32,
    /// 発走時刻 `HH:MM`（race_card 由来。欠落時 None）。
    pub post_time: Option<String>,
    /// 監視サイクル境界時刻（UTC rfc3339 秒精度 Z 終端）。1 スイープ 1 値。
    pub captured_at: &'a str,
    /// ◎の model 勝率[%]（blended の軸馬 `win_prob`×100）。
    pub axis_prob: f64,
    /// ◎の単勝オッズ（欠落時 None）。
    pub axis_win_odds: Option<f64>,
    /// ◎の複勝オッズ帯 low / high（欠落時 None。#346）。
    pub axis_place_odds_low: Option<f64>,
    pub axis_place_odds_high: Option<f64>,
    /// このレースへ配分した予算（円）。
    pub race_budget: u64,
}

/// Portfolio ＋文脈から 1 レコードを組む。`ev`・`axis` が無ければ（買い目を組めなかった）`None`。
///
/// leg は emit 粒度「1 leg = 1 組番 = 1 点」（SPA 側 `groupLegs` が券種×方式へ再グルーピングする）。
/// `verdict` は DB 契約どおり ROI≥100% を `bet`・それ未満を `skip`。`roi` 列は % 表現（`ev.roi*100`）。
pub fn build_snapshot_record(
    portfolio: &Portfolio,
    ctx: &SnapshotContext,
) -> Option<LiveEvSnapshotRecord> {
    let ev = portfolio.ev.as_ref()?;
    let axis = portfolio.axis?;

    let legs: Vec<SlipLegRecord> = portfolio
        .bets
        .iter()
        .filter(|b| b.stake > 0)
        .map(|b| SlipLegRecord {
            bet_type: b.combination.type_label().to_string(),
            // MVP: konsen=false のため box は組まれず、全 leg が ◎軸ながし。
            method: "nagashi".to_string(),
            axis: Some(axis.value()),
            combo: b.combination.horse_nums(),
            points: 1,
            amount: b.stake,
        })
        .collect();

    // 一部の買い目でオッズ未取得＝ROI を過小評価しうる（read/SPA が注記に使う）。
    let odds_missing = portfolio
        .bets
        .iter()
        .any(|b| b.stake > 0 && b.odds.is_none());

    Some(LiveEvSnapshotRecord {
        date: ctx.date,
        race_id: ctx.race_id.to_string(),
        venue: ctx.venue.to_string(),
        race_no: ctx.race_no,
        post_time: ctx.post_time.clone(),
        captured_at: ctx.captured_at.to_string(),
        verdict: if ev.roi >= 1.0 { "bet" } else { "skip" }.to_string(),
        roi: ev.roi * 100.0,
        konsen: false,
        axis: axis.value(),
        axis_prob: ctx.axis_prob,
        axis_win_odds: ctx.axis_win_odds,
        axis_place_odds_low: ctx.axis_place_odds_low,
        axis_place_odds_high: ctx.axis_place_odds_high,
        odds_missing,
        race_budget: ctx.race_budget,
        legs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use paddock_domain::{BetCombination, EvReport, HorseNum, Pair, PortfolioBet, Triple};

    fn h(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    fn ctx() -> SnapshotContext<'static> {
        SnapshotContext {
            date: NaiveDate::from_ymd_opt(2026, 7, 6).unwrap(),
            race_id: "202605020611",
            venue: "hakodate",
            race_no: 11,
            post_time: Some("15:35".to_string()),
            captured_at: "2026-07-06T06:20:00Z",
            axis_prob: 32.5,
            axis_win_odds: Some(2.4),
            axis_place_odds_low: Some(1.1),
            axis_place_odds_high: Some(1.4),
            race_budget: 5000,
        }
    }

    fn wide(a: u32, b: u32, stake: u64, odds: Option<f64>) -> PortfolioBet {
        PortfolioBet {
            combination: BetCombination::Wide(Pair::try_from((h(a), h(b))).unwrap()),
            stake,
            odds,
            ev: 1.2,
            hit_prob: 0.4,
        }
    }

    fn trio(a: u32, b: u32, c: u32, stake: u64) -> PortfolioBet {
        PortfolioBet {
            combination: BetCombination::Trio(Triple::try_from((h(a), h(b), h(c))).unwrap()),
            stake,
            odds: Some(8.0),
            ev: 1.5,
            hit_prob: 0.2,
        }
    }

    fn portfolio(bets: Vec<PortfolioBet>, roi: f64) -> Portfolio {
        Portfolio {
            axis: Some(h(6)),
            partners: vec![h(3), h(8)],
            total_stake: bets.iter().map(|b| b.stake).sum(),
            bets,
            ev: Some(EvReport {
                ev: 5200.0,
                roi,
                hit_prob: 0.55,
            }),
        }
    }

    #[test]
    fn maps_scalars_and_verdict_bet_when_roi_ge_1() {
        let p = portfolio(vec![wide(6, 3, 1500, Some(5.0))], 1.04);
        let r = build_snapshot_record(&p, &ctx()).unwrap();
        assert_eq!(r.race_id, "202605020611");
        assert_eq!(r.venue, "hakodate");
        assert_eq!(r.race_no, 11);
        assert_eq!(r.axis, 6);
        assert_eq!(r.axis_prob, 32.5);
        assert_eq!(r.axis_win_odds, Some(2.4));
        assert_eq!(r.axis_place_odds_low, Some(1.1));
        assert_eq!(r.axis_place_odds_high, Some(1.4));
        assert!(!r.konsen);
        // ROI 104% → bet、roi 列は % 表現。
        assert_eq!(r.verdict, "bet");
        assert!((r.roi - 104.0).abs() < 1e-9);
        assert_eq!(r.race_budget, 5000);
    }

    #[test]
    fn verdict_skip_when_roi_below_1() {
        let p = portfolio(vec![wide(6, 3, 1500, Some(5.0))], 0.92);
        let r = build_snapshot_record(&p, &ctx()).unwrap();
        assert_eq!(r.verdict, "skip");
        assert!((r.roi - 92.0).abs() < 1e-9);
    }

    #[test]
    fn legs_are_per_bet_nagashi_with_sorted_combo() {
        // stake>0 の買い目だけが 1 leg=1 点=1 組番（昇順）で写る。
        let p = portfolio(vec![wide(6, 3, 1500, Some(5.0)), trio(8, 3, 6, 2000)], 1.1);
        let r = build_snapshot_record(&p, &ctx()).unwrap();
        assert_eq!(r.legs.len(), 2);

        let w = &r.legs[0];
        assert_eq!(w.bet_type, "wide");
        assert_eq!(w.method, "nagashi");
        assert_eq!(w.axis, Some(6));
        assert_eq!(w.combo, vec![3, 6]); // 昇順
        assert_eq!(w.points, 1);
        assert_eq!(w.amount, 1500);

        let t = &r.legs[1];
        assert_eq!(t.bet_type, "trio");
        assert_eq!(t.combo, vec![3, 6, 8]); // 昇順
        assert_eq!(t.amount, 2000);
    }

    #[test]
    fn zero_stake_bets_are_excluded_from_legs() {
        let p = portfolio(
            vec![wide(6, 3, 0, Some(5.0)), wide(6, 8, 1500, Some(4.0))],
            1.1,
        );
        let r = build_snapshot_record(&p, &ctx()).unwrap();
        assert_eq!(r.legs.len(), 1);
        assert_eq!(r.legs[0].combo, vec![6, 8]);
    }

    #[test]
    fn odds_missing_true_when_a_staked_bet_lacks_odds() {
        let p = portfolio(vec![wide(6, 3, 1500, None), trio(8, 3, 6, 2000)], 1.1);
        let r = build_snapshot_record(&p, &ctx()).unwrap();
        assert!(r.odds_missing);
    }

    #[test]
    fn odds_missing_ignores_zero_stake_bets() {
        // stake=0 のオッズ欠落は ROI に影響しないので odds_missing を立てない。
        let p = portfolio(vec![wide(6, 3, 0, None), trio(8, 3, 6, 2000)], 1.1);
        let r = build_snapshot_record(&p, &ctx()).unwrap();
        assert!(!r.odds_missing);
    }

    #[test]
    fn none_when_portfolio_has_no_ev() {
        let mut p = portfolio(vec![wide(6, 3, 1500, Some(5.0))], 1.1);
        p.ev = None;
        assert!(build_snapshot_record(&p, &ctx()).is_none());
    }
}
