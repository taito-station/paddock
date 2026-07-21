use std::collections::HashMap;

use chrono::{FixedOffset, NaiveDate, NaiveDateTime, Utc};
use paddock_domain::RaceId;

use crate::error::Result;
use crate::interactor::settle::{RacePayoutOutcome, classify_payouts, settle_and_summarize};
use crate::netkeiba_race_id::netkeiba_race_id_from_paddock;
use crate::repository::{
    PredictSessionRepository, RaceCardRepository, RaceRepository, RaceResultRepository,
};
use crate::result_page_fetcher::ResultPageFetcher;

/// 同日結果取り込み＋自動精算のレポート（#381）。
///
/// 精算集計（`settled_races`〜`roi`）は `SettleReport` と同一の意味。加えて、当パスで新規に着順を
/// 取り込んで確定したレースを返す。
#[derive(Debug, Clone, PartialEq)]
pub struct RefreshReport {
    pub settled_races: usize,
    pub pending_races: usize,
    pub voided_races: usize,
    pub refunded_bets: usize,
    pub total_bet: u64,
    pub total_payout: u64,
    pub balance: u64,
    pub roi: Option<f64>,
    /// 当パスで新規に着順を取り込んで確定したレース数。
    pub newly_confirmed_races: usize,
    /// 当パスで新規に確定したレースの `race_id`（昇順）。
    pub confirmed_race_ids: Vec<RaceId>,
}

/// 同日のレース結果（着順・確定払戻）を取り込み、UI へ自動反映するための状態を作るユースケース（#381）。
///
/// レースごとに結果ページを **1 回だけ** 取得して着順（`results` へ upsert）と払戻（in-memory で精算）を
/// 得る。`settle_session`（#40）のような払戻の再取得はしない。精算集計は `SettleInteractor` と共有する
/// `settle_and_summarize` を再利用し、精算エンジンを二重化しない。冪等（未確定のみ対象・確定済みは
/// netkeiba を叩かない）。
pub struct ResultsInteractor<S: ResultPageFetcher, R> {
    pub scraper: S,
    pub repository: R,
}

impl<S, R> ResultsInteractor<S, R>
where
    S: ResultPageFetcher,
    R: RaceRepository + RaceCardRepository + PredictSessionRepository + RaceResultRepository,
{
    pub fn new(scraper: S, repository: R) -> Self {
        Self {
            scraper,
            repository,
        }
    }

    /// 指定日の結果を取り込み、セッションがあれば精算する。
    ///
    /// 対象は「発走済み（`post_time` 経過）かつ未確定」のレース。`force=true` は post_time gating を
    /// 緩和し、`post_time` 未取得（#391 で対象外）の未確定レースも取得対象にする（手動フォールバック）。
    /// 確定済み（`results` に着順あり）は netkeiba を叩かずスキップする。
    pub async fn refresh(&self, date: NaiveDate, force: bool) -> Result<RefreshReport> {
        let post_times = self.repository.find_post_times_by_date(date).await?;
        let confirmed = self.repository.find_result_confirmed_by_date(date).await?;
        // 候補ユニバース = 当日の全レース（races ∪ race_cards）。post_time 欠損レースも含むため、
        // force での救済対象に入る。
        let universe = self.repository.find_races_by_date(date).await?;

        // JST 現在時刻（netkeiba/開催は JST）。post_datetime 以降を「発走済み」とみなす。
        let now_naive = Utc::now()
            .with_timezone(&FixedOffset::east_opt(9 * 3600).expect("valid JST offset"))
            .naive_local();

        let mut outcome_by_race: HashMap<String, RacePayoutOutcome> = HashMap::new();
        let mut confirmed_race_ids: Vec<RaceId> = Vec::new();

        for race in &universe {
            let race_id = &race.race_id;
            if confirmed.get(race_id).copied().unwrap_or(false) {
                continue; // 確定済みは netkeiba を叩かない。
            }
            let started = match post_times.get(race_id) {
                Some(t) => now_naive >= NaiveDateTime::new(date, *t),
                None => false, // post_time 未取得は「発走済みと断定しない」（#391）。
            };
            if !force && !started {
                continue; // 未発走（force なら gating を緩和）。
            }

            let netkeiba_id = netkeiba_race_id_from_paddock(race_id)?;
            // fetch_race_result_page は async（#458）。api-server 経路では実装が spawn_blocking へ
            // 逃がすため、未確定レース数ぶんの直列 sleep+GET で actix worker を塞がない。
            let (rows, payouts) = match self.scraper.fetch_race_result_page(&netkeiba_id).await {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!(
                        race_id = race_id.value(),
                        error = %e,
                        "結果ページの取得に失敗。pending として継続"
                    );
                    continue; // 取得失敗 = pending 据え置き。
                }
            };

            // 着順が 1 件も無い（結果ページ未生成・中止で成績表なし）: 着順を書かず pending 据え置き。
            // 払戻ブロックの有無（Voided/Pending）は精算入力として記録し、全額返還レースは精算に反映する。
            if !rows.is_empty() {
                // gate_num/horse_name の補完に出馬表が要る。無ければ着順を書かず pending。
                let Some(card) = self.repository.find_race_card(race_id).await? else {
                    tracing::warn!(
                        race_id = race_id.value(),
                        "race_card が無く gate_num/horse_name を補完できないため着順を書かない"
                    );
                    outcome_by_race.insert(race_id.value().to_string(), classify_payouts(payouts));
                    continue;
                };
                // 破壊的上書き・DELETE をしない専用 upsert（races メタ・他馬の既存着順を温存）。
                self.repository.upsert_results(&card, &rows).await?;
                confirmed_race_ids.push(race_id.clone());
            }

            outcome_by_race.insert(race_id.value().to_string(), classify_payouts(payouts));
        }

        confirmed_race_ids.sort_by(|a, b| a.value().cmp(b.value()));
        let newly_confirmed_races = confirmed_race_ids.len();

        // セッションがあれば精算する（取得済み payouts を in-memory で使い、netkeiba を再取得しない）。
        let Some(mut session) = self.repository.find_predict_session(date).await? else {
            return Ok(RefreshReport {
                settled_races: 0,
                pending_races: 0,
                voided_races: 0,
                refunded_bets: 0,
                total_bet: 0,
                total_payout: 0,
                balance: 0,
                roi: None,
                newly_confirmed_races,
                confirmed_race_ids,
            });
        };

        let mut bets = self.repository.find_predict_bets_with_id(date).await?;

        // 当パスで払戻を取得していない bet レースのうち、確定済み（今パス確定 or 既確定）は
        // AlreadySettled（payout 据え置き・settled 算入）にする。未確定は Pending のまま。
        let newly_confirmed: std::collections::HashSet<String> = confirmed_race_ids
            .iter()
            .map(|r| r.value().to_string())
            .collect();
        for (_, bet) in bets.iter() {
            let key = bet.race_id.value().to_string();
            if outcome_by_race.contains_key(&key) {
                continue;
            }
            let is_confirmed =
                newly_confirmed.contains(&key) || confirmed.contains_key(&bet.race_id);
            if is_confirmed {
                outcome_by_race.insert(key, RacePayoutOutcome::AlreadySettled);
            }
        }

        let summary = if bets.is_empty() {
            None
        } else {
            let s = settle_and_summarize(&mut session, &mut bets, &outcome_by_race);
            self.repository
                .settle_predict_session(&session, &s.updated)
                .await?;
            Some(s)
        };

        let (settled_races, pending_races, voided_races, refunded_bets) = match &summary {
            Some(s) => (
                s.settled_races,
                s.pending_races,
                s.voided_races,
                s.refunded_bets,
            ),
            None => (0, 0, 0, 0),
        };

        Ok(RefreshReport {
            settled_races,
            pending_races,
            voided_races,
            refunded_bets,
            total_bet: session.total_bet,
            total_payout: session.total_payout,
            balance: session.balance,
            roi: roi(session.total_bet, session.total_payout),
            newly_confirmed_races,
            confirmed_race_ids,
        })
    }
}

/// 回収率(%)。総賭け金 0 なら `None`。
fn roi(total_bet: u64, total_payout: u64) -> Option<f64> {
    if total_bet == 0 {
        None
    } else {
        Some(total_payout as f64 / total_bet as f64 * 100.0)
    }
}
