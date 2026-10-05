//! 過去の開催日の買い目を、朝と T-40 の 2 時点で再構成する（#724）。
//!
//! 朝 = 当日 JST 0 時以降で最初に単勝を取ったスナップショット、T-40 = predict-watch の初回スイープ
//! （`live_ev_snapshots` の最古行の `captured_at`）の後に保存された全券種のスナップショット。
//! どちらも本番と同じ [`compose_portfolio`]（＝`build_portfolio`）で組むので、買い方を二重に実装しない
//! （ADR 0064）。本番の経路（predict / predict-watch / api）は触らない。

use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::{
    EstimationConfig, HorseProbability, PinnedSelection, Portfolio, RaceId, RaceOdds,
};

use super::orchestrate::{PredictionViews, compose_portfolio};
use crate::error::Result;
use crate::interactor::Interactor;
use crate::repository::{
    LiveEvPin, LiveEvRepository, OddsRepository, RaceCardRepository, SnapshotPoint, StatsRepository,
};

/// 1 レースの 2 時点の買い目（[`assemble_slips`] の戻り値）。
pub struct AssembledSlips {
    /// T-40 の盤で、固定なしに選んだ買い目（＝現行の T-40 固定と同じ選定）。
    pub t40: Portfolio,
    /// 朝の盤で選んだ軸・相手・混戦（記録側の固定と同じ規則で伝票から読む）。
    pub morning_pick: PinnedSelection,
    /// 朝の選定を固定して、T-40 の盤（その時点の出走馬・オッズ）で組んだ買い目。
    /// 朝に固定してから取消が出た場合は、固定の規則（相手は落とすだけで補充しない・軸が取消なら
    /// ライブの首位へ戻す, REQ-D23-007）がそのまま効く。
    pub morning_fixed: Portfolio,
}

/// 朝と T-40 の推定・オッズから、2 時点の買い目を組む（#724）。
pub fn assemble_slips(
    morning: (&PredictionViews, &RaceOdds),
    t40: (&PredictionViews, &RaceOdds),
    race_budget: u64,
) -> AssembledSlips {
    let morning_portfolio = compose_portfolio(
        morning.0,
        morning.1,
        race_budget,
        &PinnedSelection::default(),
    );
    let morning_pick = PinnedSelection::from_staked_legs(&morning_portfolio);
    AssembledSlips {
        t40: compose_portfolio(t40.0, t40.1, race_budget, &PinnedSelection::default()),
        morning_fixed: compose_portfolio(t40.0, t40.1, race_budget, &morning_pick),
        morning_pick,
    }
}

/// 1 レースの再構成結果（[`Interactor::reconstruct_race_slips`]）。
pub struct ReconstructedRace {
    /// 記録された初回スイープの固定（`live_ev_snapshots` の最古行）。再構成の検査に使う。
    pub recorded: LiveEvPin,
    /// 朝に使ったスナップショットの取得時刻（UTC rfc3339）。
    pub morning_at: Option<String>,
    /// T-40 に使ったスナップショットの取得時刻（UTC rfc3339）。
    pub t40_at: Option<String>,
    /// 2 時点の買い目。組めなかったレースは `None`（理由は `skip`）。
    pub slips: Option<AssembledSlips>,
    /// T-40 の blended 確率（再構成の検査で、記録の軸の勝率と突き合わせるために出す）。
    pub t40_blended: Vec<HorseProbability>,
    /// 組めなかった理由（スナップショットが無い・推定に失敗した等）。
    pub skip: Option<String>,
}

impl<R: StatsRepository + RaceCardRepository + OddsRepository + LiveEvRepository> Interactor<R> {
    /// `date` の開催日について、predict-watch が初回スイープで固定した各レースを、朝と T-40 の
    /// 2 時点で再構成する（#724）。統計は `date` より前の結果だけで作り、確率推定は本番設定
    /// （[`EstimationConfig::production`]）を使う。1 レースの失敗では止めず、理由を付けて返す。
    pub async fn reconstruct_race_slips(
        &self,
        date: NaiveDate,
        blend_alpha: Option<f64>,
        race_budget: u64,
    ) -> Result<Vec<ReconstructedRace>> {
        let config = EstimationConfig::production();
        // 朝 = 当日 JST 0 時以降（前日夜の fetch-card の薄いオッズを拾わない）
        let morning_since = jst_midnight_utc(date);
        let mut out = Vec::new();
        for pin in self.repository.find_live_ev_pins_by_date(date).await? {
            let mut rec = ReconstructedRace {
                recorded: pin.clone(),
                morning_at: None,
                t40_at: None,
                slips: None,
                t40_blended: Vec::new(),
                skip: None,
            };
            match self
                .reconstruct_one(&pin, date, morning_since, blend_alpha, race_budget, &config)
                .await
            {
                Ok(Ok((morning_at, t40_at, slips, t40_blended))) => {
                    rec.morning_at = Some(morning_at);
                    rec.t40_at = Some(t40_at);
                    rec.slips = Some(slips);
                    rec.t40_blended = t40_blended;
                }
                Ok(Err(reason)) => rec.skip = Some(reason),
                Err(e) => rec.skip = Some(format!("error: {e}")),
            }
            out.push(rec);
        }
        Ok(out)
    }

    /// 1 レース分。外側の `Err` は DB 等の失敗、内側の `Err` はスナップショットが無いなどの「組めない」理由。
    async fn reconstruct_one(
        &self,
        pin: &LiveEvPin,
        date: NaiveDate,
        morning_since: DateTime<Utc>,
        blend_alpha: Option<f64>,
        race_budget: u64,
        config: &EstimationConfig,
    ) -> Result<std::result::Result<(String, String, AssembledSlips, Vec<HorseProbability>), String>>
    {
        let Ok(race_id) = RaceId::try_from(pin.race_id.as_str()) else {
            return Ok(Err(format!("race_id を解釈できない: {}", pin.race_id)));
        };
        let Ok(captured) = DateTime::parse_from_rfc3339(&pin.captured_at) else {
            return Ok(Err(format!(
                "captured_at を解釈できない: {}",
                pin.captured_at
            )));
        };
        let captured = captured.with_timezone(&Utc);
        let Some(morning) = self
            .repository
            .find_race_odds_snapshot(&race_id, SnapshotPoint::FirstWinSince(morning_since))
            .await?
        else {
            return Ok(Err("朝のスナップショットが無い".to_string()));
        };
        let Some(t40) = self
            .repository
            .find_race_odds_snapshot(&race_id, SnapshotPoint::FirstCompleteSince(captured))
            .await?
        else {
            return Ok(Err(
                "初回スイープ以降に全券種のスナップショットが無い".to_string()
            ));
        };
        let views_m = self
            .predict_race_views_at(&race_id, blend_alpha, &morning.odds, date, config)
            .await?;
        let views_t = self
            .predict_race_views_at(&race_id, blend_alpha, &t40.odds, date, config)
            .await?;
        let slips = assemble_slips(
            (&views_m, &morning.odds),
            (&views_t, &t40.odds),
            race_budget,
        );
        Ok(Ok((
            morning.fetched_at,
            t40.fetched_at,
            slips,
            views_t.blended,
        )))
    }
}

/// 開催日 `date` の JST 0 時を UTC で表す。
fn jst_midnight_utc(date: NaiveDate) -> DateTime<Utc> {
    let midnight = date.and_hms_opt(0, 0, 0).expect("0 時は常に有効");
    DateTime::<Utc>::from_naive_utc_and_offset(midnight, Utc) - chrono::Duration::hours(9)
}

#[cfg(test)]
mod tests {
    use paddock_domain::{BetType, HorseName, HorseNum, HorseProbability, OddsValue, RaceId};

    use super::*;
    use crate::interactor::race::predict::RecentRunsCoverage;

    fn horse(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    fn views(wins: &[(u32, f64)]) -> PredictionViews {
        let probs: Vec<HorseProbability> = wins
            .iter()
            .map(|(n, w)| HorseProbability {
                horse_num: horse(*n),
                horse_name: HorseName::try_from(format!("ウマ{n}")).unwrap(),
                win_prob: *w,
                place_prob: 0.0,
                show_prob: 0.0,
            })
            .collect();
        PredictionViews {
            blended: probs.clone(),
            pure: probs,
            explanations: Vec::new(),
            recent_runs_coverage: RecentRunsCoverage {
                field_size: wins.len(),
                horses_with_runs: wins.len(),
            },
        }
    }

    fn win_only(nums: &[u32]) -> RaceOdds {
        let mut o = RaceOdds::empty(RaceId::try_from("2026-2-niigata-3-10R").unwrap());
        for n in nums {
            o.win
                .insert(horse(*n), OddsValue::try_from((BetType::Win, 5.0)).unwrap());
        }
        o
    }

    #[test]
    fn jst_midnight_is_previous_day_15_utc() {
        let d = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap();
        assert_eq!(
            jst_midnight_utc(d).to_rfc3339(),
            "2026-08-14T15:00:00+00:00"
        );
    }

    #[test]
    fn morning_pick_is_fixed_while_t40_follows_its_own_ranking() {
        // 朝は 1 番が首位、T-40 では 2 番が首位に入れ替わった。
        let m = views(&[
            (1, 0.40),
            (2, 0.20),
            (3, 0.15),
            (4, 0.10),
            (5, 0.08),
            (6, 0.07),
        ]);
        let t = views(&[
            (2, 0.40),
            (1, 0.20),
            (3, 0.15),
            (4, 0.10),
            (6, 0.08),
            (5, 0.07),
        ]);
        let odds = win_only(&[1, 2, 3, 4, 5, 6]);
        let s = assemble_slips((&m, &odds), (&t, &odds), 5000);
        assert_eq!(s.t40.axis, Some(horse(2)), "T-40 は T-40 の首位");
        assert_eq!(s.morning_pick.axis, Some(horse(1)), "朝の選定は朝の首位");
        assert_eq!(
            s.morning_fixed.axis,
            Some(horse(1)),
            "朝の固定は T-40 の盤でも朝の軸"
        );
        // 固定した相手は T-40 の勝率順に並べ直される（build_portfolio の仕様）ので集合で比べる
        let sorted = |mut v: Vec<HorseNum>| {
            v.sort_by_key(|h| h.value());
            v
        };
        assert_eq!(
            sorted(s.morning_fixed.partners.clone()),
            sorted(s.morning_pick.partners.clone().unwrap()),
            "相手も朝の選定のまま"
        );
    }

    #[test]
    fn morning_fixed_drops_scratched_partner_without_backfill() {
        // 朝の相手に入っていた 3 番が、T-40 の盤には載っていない（取消）。
        let m = views(&[
            (1, 0.40),
            (2, 0.20),
            (3, 0.15),
            (4, 0.10),
            (5, 0.08),
            (6, 0.07),
        ]);
        let t = views(&[
            (1, 0.40),
            (2, 0.20),
            (4, 0.15),
            (5, 0.10),
            (6, 0.08),
            (7, 0.07),
        ]);
        let s = assemble_slips(
            (&m, &win_only(&[1, 2, 3, 4, 5, 6])),
            (&t, &win_only(&[1, 2, 4, 5, 6, 7])),
            5000,
        );
        let picked = s.morning_pick.partners.clone().unwrap();
        assert!(picked.contains(&horse(3)));
        assert!(
            !s.morning_fixed.partners.contains(&horse(3)),
            "取消の相手は落とす"
        );
        assert!(
            !s.morning_fixed.partners.contains(&horse(7)),
            "朝の選定に無い馬で補充しない"
        );
        assert_eq!(s.morning_fixed.partners.len(), picked.len() - 1);
    }
}
