use std::collections::{BTreeMap, HashMap};

use chrono::{NaiveDate, Utc};
use paddock_domain::{RaceId, RacePayouts, settle_bet};

use crate::error::{Error, Result};
use crate::netkeiba_race_id::netkeiba_race_id_from_paddock;
use crate::payout_fetcher::PayoutFetcher;
use crate::repository::{PredictBetRecord, PredictSessionRecord, PredictSessionRepository};

/// 1 レースぶんの確定払戻の取得結果（精算集計の入力）。同日取り込み（`ResultsInteractor`）と
/// 手動精算（`SettleInteractor`）で共有する精算コアの入力表現。
pub enum RacePayoutOutcome {
    /// 確定（払戻ブロックあり）。各 bet を [`settle_bet`] で照合する。
    Settled(RacePayouts),
    /// 開催中止・全馬取消（払戻ブロック無し かつ 全出走馬が取消/除外）。全買い目を全額返還（#131）。
    Voided,
    /// 未確定（払戻ブロック無し）・取得失敗。payout 据え置きで pending。
    Pending,
    /// 既に確定済みで、当パスでは払戻を取得していないレース（#381 同日取り込みの再実行）。
    /// bet の payout は前パスで確定済みなので触らず、pending には数えず settled として扱う。
    AlreadySettled,
}

/// 精算集計の結果（永続化は呼び出し側）。
pub struct SettlementSummary {
    /// 確定した bet の `(bet_id, payout)`（書き込み対象）。
    pub updated: Vec<(i64, u64)>,
    pub settled_races: usize,
    pub pending_races: usize,
    pub voided_races: usize,
    pub refunded_bets: usize,
}

/// 買い目を race 別の確定払戻で精算し、セッション集計（total_payout・balance・completed）を
/// **ゼロから再計算**して更新する共有コア（冪等・#40/#131/#381）。`bets` の payout を上書きし、
/// `outcome_by_race` にキーが無いレースは `Pending`（payout 据え置き）として扱う。永続化は呼び出し側。
pub fn settle_and_summarize(
    session: &mut PredictSessionRecord,
    bets: &mut [(i64, PredictBetRecord)],
    outcome_by_race: &HashMap<String, RacePayoutOutcome>,
) -> SettlementSummary {
    // race_id 別に bet index を集約（BTreeMap で race_id 昇順に処理し、出力を安定させる）。
    let mut by_race: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, (_, bet)) in bets.iter().enumerate() {
        by_race
            .entry(bet.race_id.value().to_string())
            .or_default()
            .push(idx);
    }

    let mut settled_races = 0usize;
    let mut pending_races = 0usize;
    let mut voided_races = 0usize;
    let mut refunded_bets = 0usize;
    let mut updated: Vec<(i64, u64)> = Vec::new();

    for (race_key, idxs) in &by_race {
        match outcome_by_race.get(race_key) {
            None | Some(RacePayoutOutcome::Pending) => {
                // 未確定・取得失敗: payout 据え置きで pending。
                pending_races += 1;
            }
            Some(RacePayoutOutcome::AlreadySettled) => {
                // 既に確定・精算済み: payout 据え置きで settled（完了判定に算入・再取得しない）。
                settled_races += 1;
            }
            Some(RacePayoutOutcome::Voided) => {
                // 開催中止・全馬取消: 全買い目を全額返還する（#131）。
                voided_races += 1;
                for &idx in idxs {
                    let (bet_id, bet) = &bets[idx];
                    refunded_bets += 1;
                    updated.push((*bet_id, bet.stake));
                    bets[idx].1.payout = bet.stake;
                }
            }
            Some(RacePayoutOutcome::Settled(payouts)) => {
                settled_races += 1;
                for &idx in idxs {
                    let (bet_id, bet) = &bets[idx];
                    // 返還判定と払戻額を 1 度の評価でまとめて受け取る（is_refunded の二重評価を避ける）。
                    let settlement =
                        settle_bet(&bet.bet_type, &bet.combination, bet.stake, payouts);
                    if settlement.is_refund() {
                        refunded_bets += 1;
                    }
                    let payout = settlement.payout();
                    updated.push((*bet_id, payout));
                    bets[idx].1.payout = payout;
                }
            }
        }
    }

    // セッション集計をゼロから再計算する（冪等）。total_bet は購入時に確定済みのため据え置き。
    let total_payout: u64 = bets.iter().map(|(_, b)| b.payout).sum();
    // 購入は残高ガード下で行われ total_bet <= budget が不変条件。万一崩れた場合 saturating_sub は
    // 0 に張り付いて異常を隠すため、debug ビルドで早期検知する（本番は防御的に飽和維持）。
    debug_assert!(
        session.total_bet <= session.budget,
        "total_bet ({}) must not exceed budget ({})",
        session.total_bet,
        session.budget
    );
    let balance = session
        .budget
        .saturating_sub(session.total_bet)
        .saturating_add(total_payout);
    session.total_payout = total_payout;
    session.balance = balance;
    session.completed = pending_races == 0;
    session.updated_at = Utc::now();

    SettlementSummary {
        updated,
        settled_races,
        pending_races,
        voided_races,
        refunded_bets,
    }
}

/// 取得済み `RacePayouts` を精算コアの入力へ分類する。払戻ブロック有り→`Settled`、
/// 空 かつ 全馬取消/中止→`Voided`（#131）、空→`Pending`（未確定・payout 据え置き）。
pub fn classify_payouts(payouts: RacePayouts) -> RacePayoutOutcome {
    if payouts.is_empty() {
        if payouts.is_fully_refunded() {
            RacePayoutOutcome::Voided
        } else {
            RacePayoutOutcome::Pending
        }
    } else {
        RacePayoutOutcome::Settled(payouts)
    }
}

/// 確定払戻の自動精算ユースケース（#40）。
///
/// 予想セッションの購入済み買い目（`predict_bets`）を netkeiba の確定払戻と照合して payout を
/// セットし、セッションの総払戻・収支・回収率を更新する。**毎回ゼロから再計算**するため
/// 再実行で二重加算しない（冪等）。未確定レースは payout を据え置いて pending とする。
/// `PayoutFetcher`/`PredictSessionRepository` を必要とするため、メイン `Interactor` には載せず専用 interactor
/// として切り出す（`OddsInteractor` と同方針）。
pub struct SettleInteractor<S: PayoutFetcher, R> {
    pub scraper: S,
    pub repository: R,
}

impl<S: PayoutFetcher, R: PredictSessionRepository> SettleInteractor<S, R> {
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
    ///    開催中止・全馬取消（払戻ブロック無し かつ 全馬取消/除外）は全買い目を全額返還する（#131）。
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
                voided_races: 0,
                refunded_bets: 0,
                total_bet: session.total_bet,
                total_payout: session.total_payout,
                balance: session.balance,
                roi: roi(session.total_bet, session.total_payout),
            });
        }

        // レース別に確定払戻を取得し、精算コアの入力（race_key → RacePayoutOutcome）へ落とす。
        // 取得失敗（ネット断・BAN 等）はマップに入れず、コア側で Pending（payout 据え置き）に倒す。
        // 1 レースの失敗で確定済みの他レースまで巻き添えに未保存（永続化はループ後の 1 TXN）にしない。
        let race_keys: Vec<String> = {
            let mut set = BTreeMap::new();
            for (_, bet) in bets.iter() {
                set.insert(bet.race_id.value().to_string(), ());
            }
            set.into_keys().collect()
        };
        let mut outcome_by_race: HashMap<String, RacePayoutOutcome> = HashMap::new();
        for race_key in &race_keys {
            let race_id = RaceId::try_from(race_key.as_str())?;
            let netkeiba_id = netkeiba_race_id_from_paddock(&race_id)?;
            match self.scraper.fetch_race_payouts(&netkeiba_id) {
                Ok(payouts) => {
                    outcome_by_race.insert(race_key.clone(), classify_payouts(payouts));
                }
                Err(e) => {
                    tracing::warn!(
                        race_id = race_id.value(),
                        error = %e,
                        "確定払戻の取得に失敗。pending として継続"
                    );
                    // マップに入れない = Pending 扱い。
                }
            }
        }

        let summary = settle_and_summarize(&mut session, &mut bets, &outcome_by_race);

        self.repository
            .settle_predict_session(&session, &summary.updated)
            .await?;

        Ok(SettleReport {
            settled_races: summary.settled_races,
            pending_races: summary.pending_races,
            voided_races: summary.voided_races,
            refunded_bets: summary.refunded_bets,
            total_bet: session.total_bet,
            total_payout: session.total_payout,
            balance: session.balance,
            roi: roi(session.total_bet, session.total_payout),
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
    /// 開催中止・全馬取消で全買い目を全額返還したレース数（#131）。
    pub voided_races: usize,
    /// 返還（取消/除外馬を含む組番、または開催中止・全馬取消）として stake を全額返戻した買い目数。
    pub refunded_bets: usize,
    pub total_bet: u64,
    pub total_payout: u64,
    pub balance: u64,
    /// 回収率(%)。総賭け金 0 なら `None`。
    pub roi: Option<f64>,
}
