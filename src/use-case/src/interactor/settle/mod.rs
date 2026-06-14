use std::collections::BTreeMap;

use chrono::{NaiveDate, Utc};
use paddock_domain::{RaceId, settle_bet};

use crate::error::{Error, Result};
use crate::netkeiba_race_id::netkeiba_race_id_from_paddock;
use crate::payout_fetcher::PayoutFetcher;
use crate::repository::Repository;

/// 確定払戻の自動精算ユースケース（#40）。
///
/// 予想セッションの購入済み買い目（`predict_bets`）を netkeiba の確定払戻と照合して payout を
/// セットし、セッションの総払戻・収支・回収率を更新する。**毎回ゼロから再計算**するため
/// 再実行で二重加算しない（冪等）。未確定レースは payout を据え置いて pending とする。
/// `PayoutFetcher`/`Repository` を必要とするため、メイン `Interactor` には載せず専用 interactor
/// として切り出す（`OddsInteractor` と同方針）。
pub struct SettleInteractor<S: PayoutFetcher, R: Repository> {
    pub scraper: S,
    pub repository: R,
}

impl<S: PayoutFetcher, R: Repository> SettleInteractor<S, R> {
    pub fn new(scraper: S, repository: R) -> Self {
        Self {
            scraper,
            repository,
        }
    }

    /// 指定日のセッションを確定払戻で精算する。
    ///
    /// 1. セッションが無ければエラー。買い目が無ければ空の report を返す。
    /// 2. レース毎に確定払戻を取得。未確定（払戻ブロック無し）なら pending として payout 据え置き。
    /// 3. 確定レースの各 bet を `settle_bet` で再計算し、payout を上書きする。
    /// 4. `total_payout = Σ payout`・`balance = budget - total_bet + total_payout` を再計算。
    /// 5. 全購入レース確定なら `completed = true`。1 トランザクションで永続化する。
    pub async fn settle_session(&self, date: NaiveDate) -> Result<SettleReport> {
        let mut session = self
            .repository
            .find_predict_session(date)
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "{} のセッションがありません",
                    date.format("%Y-%m-%d")
                ))
            })?;

        let mut bets = self.repository.find_predict_bets_with_id(date).await?;
        if bets.is_empty() {
            return Ok(SettleReport {
                settled_races: 0,
                pending_races: 0,
                total_bet: session.total_bet,
                total_payout: session.total_payout,
                balance: session.balance,
                roi: roi(session.total_bet, session.total_payout),
            });
        }

        // race_id 別に bet を集約（BTreeMap で race_id 昇順に処理し、出力を安定させる）。
        let mut by_race: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (idx, (_, bet)) in bets.iter().enumerate() {
            by_race
                .entry(bet.race_id.value().to_string())
                .or_default()
                .push(idx);
        }

        let mut settled_races = 0usize;
        let mut pending_races = 0usize;
        // 確定したレースの bet の (bet_id, payout) のみを書き込み対象にする。
        let mut updated: Vec<(i64, u64)> = Vec::new();

        for (race_key, idxs) in &by_race {
            let race_id = RaceId::try_from(race_key.as_str())?;
            let netkeiba_id = netkeiba_race_id_from_paddock(&race_id)?;
            let payouts = self.scraper.fetch_race_payouts(&netkeiba_id)?;
            if payouts.is_empty() {
                // 未確定: payout 据え置きで pending。
                pending_races += 1;
                continue;
            }
            settled_races += 1;
            for &idx in idxs {
                let (bet_id, bet) = &bets[idx];
                let payout = settle_bet(&bet.bet_type, &bet.combination, bet.stake, &payouts);
                updated.push((*bet_id, payout));
                bets[idx].1.payout = payout;
            }
        }

        // セッション集計をゼロから再計算する（冪等）。total_bet は購入時に確定済みのため据え置き。
        let total_payout: u64 = bets.iter().map(|(_, b)| b.payout).sum();
        let balance = session
            .budget
            .saturating_sub(session.total_bet)
            .saturating_add(total_payout);
        session.total_payout = total_payout;
        session.balance = balance;
        session.completed = pending_races == 0;
        session.updated_at = Utc::now();

        self.repository
            .settle_predict_session(&session, &updated)
            .await?;

        Ok(SettleReport {
            settled_races,
            pending_races,
            total_bet: session.total_bet,
            total_payout,
            balance,
            roi: roi(session.total_bet, total_payout),
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

/// 自動精算の結果サマリ（CLI 表示用）。
#[derive(Debug, Clone, PartialEq)]
pub struct SettleReport {
    /// 確定して payout を更新したレース数。
    pub settled_races: usize,
    /// 未確定でスキップしたレース数（payout 据え置き）。
    pub pending_races: usize,
    pub total_bet: u64,
    pub total_payout: u64,
    pub balance: u64,
    /// 回収率(%)。総賭け金 0 なら `None`。
    pub roi: Option<f64>,
}
