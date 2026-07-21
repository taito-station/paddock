//! SettleInteractor の単体テスト。
//!
//! in-memory モックの Repository / PayoutFetcher で、確定払戻による payout 再計算・
//! 未確定レースの pending スキップ・冪等性（再実行で値が変わらない）を検証する。

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::{RaceId, RacePayouts};
use paddock_use_case::repository::{
    PredictBetRecord, PredictRaceConditionRecord, PredictSessionRecord, PredictSessionRepository,
};
use paddock_use_case::{Error, PayoutFetcher, Result, SettleInteractor};

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 6, 14).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-06-14T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn bet(race_id: &str, bet_type: &str, combo: &str, stake: u64, payout: u64) -> PredictBetRecord {
    PredictBetRecord {
        race_id: RaceId::try_from(race_id).unwrap(),
        bet_type: bet_type.to_string(),
        combination: combo.to_string(),
        stake,
        payout,
        ev: 1.5,
    }
}

// --- モック PayoutFetcher --------------------------------------------------

/// netkeiba_race_id → RacePayouts のマップを返すフェイク。未登録は空（未確定）扱い。
struct FakeFetcher {
    payouts: HashMap<String, RacePayouts>,
}

impl PayoutFetcher for FakeFetcher {
    fn fetch_race_payouts(&self, netkeiba_race_id: &str) -> Result<RacePayouts> {
        Ok(self
            .payouts
            .get(netkeiba_race_id)
            .cloned()
            .unwrap_or_else(|| RacePayouts::empty(RaceId::try_from("2026-1-tokyo-1-1R").unwrap())))
    }
}

// --- モック Repository -----------------------------------------------------

/// settle_predict_session が受け取った (session, settled bet 群) のキャプチャ。
type WrittenCapture = Option<(PredictSessionRecord, Vec<(i64, u64)>)>;

struct MockRepo {
    session: PredictSessionRecord,
    bets: Vec<(i64, PredictBetRecord)>,
    /// settle_predict_session で受け取った (session, settled) を記録する。
    written: Mutex<WrittenCapture>,
}

impl PredictSessionRepository for MockRepo {
    async fn find_predict_session(&self, _: NaiveDate) -> Result<Option<PredictSessionRecord>> {
        Ok(Some(self.session.clone()))
    }

    async fn find_predict_bets_with_id(
        &self,
        _: NaiveDate,
    ) -> Result<Vec<(i64, PredictBetRecord)>> {
        Ok(self.bets.clone())
    }

    async fn settle_predict_session(
        &self,
        session: &PredictSessionRecord,
        settled: &[(i64, u64)],
    ) -> Result<()> {
        *self.written.lock().unwrap() = Some((session.clone(), settled.to_vec()));
        Ok(())
    }

    // --- 以降は settle で未使用 ---
    async fn find_predict_bets(&self, _: NaiveDate) -> Result<Vec<PredictBetRecord>> {
        unimplemented!()
    }
    async fn save_predict_session(&self, _: &PredictSessionRecord) -> Result<()> {
        unimplemented!()
    }
    async fn save_race_outcome(
        &self,
        _: NaiveDate,
        _: &RaceId,
        _: &[PredictBetRecord],
        _: DateTime<Utc>,
    ) -> Result<PredictSessionRecord> {
        unimplemented!()
    }
    async fn find_predict_race_conditions(
        &self,
        _: NaiveDate,
    ) -> Result<Vec<PredictRaceConditionRecord>> {
        unimplemented!()
    }
    async fn save_predict_race_condition(
        &self,
        _: NaiveDate,
        _: &PredictRaceConditionRecord,
        _: DateTime<Utc>,
    ) -> Result<()> {
        unimplemented!()
    }
}

// --- fixtures --------------------------------------------------------------

fn session(budget: u64, total_bet: u64) -> PredictSessionRecord {
    PredictSessionRecord {
        date: date(),
        budget,
        balance: budget - total_bet,
        total_bet,
        total_payout: 0,
        completed: false,
        created_at: now(),
        updated_at: now(),
    }
}

/// 1R(netkeiba 202605040101) を 1 件、2R を 1 件購入したセッション。
/// race_id は paddock 形式（netkeiba_race_id_from_paddock の逆変換が成立する形）。
fn two_race_setup(
    r1_payouts: Option<RacePayouts>,
    r2_payouts: Option<RacePayouts>,
) -> (MockRepo, FakeFetcher) {
    let r1 = "2026-4-tokyo-1-1R"; // netkeiba 202605040101
    let r2 = "2026-4-tokyo-1-2R"; // netkeiba 202605040102
    let bets = vec![
        (1i64, bet(r1, "win", "8", 1000, 0)),
        (2i64, bet(r2, "quinella", "6-8", 500, 0)),
    ];
    // total_bet = 1500、予算 10000。
    let repo = MockRepo {
        session: session(10000, 1500),
        bets,
        written: Mutex::new(None),
    };
    let mut payouts = HashMap::new();
    if let Some(p) = r1_payouts {
        payouts.insert("202605040101".to_string(), p);
    }
    if let Some(p) = r2_payouts {
        payouts.insert("202605040102".to_string(), p);
    }
    (repo, FakeFetcher { payouts })
}

fn payouts_win8_140() -> RacePayouts {
    let mut p = RacePayouts::empty(RaceId::try_from("2026-4-tokyo-1-1R").unwrap());
    p.insert("win", "8", 140);
    p
}

fn payouts_quinella_1260() -> RacePayouts {
    let mut p = RacePayouts::empty(RaceId::try_from("2026-4-tokyo-1-2R").unwrap());
    p.insert("quinella", "6-8", 1260);
    p
}

/// 開催中止・全馬取消（払戻ブロック無し・全額返還レース）。
fn payouts_voided(race_id: &str) -> RacePayouts {
    let mut p = RacePayouts::empty(RaceId::try_from(race_id).unwrap());
    p.mark_fully_refunded();
    p
}

// --- tests -----------------------------------------------------------------

#[tokio::test]
async fn settles_all_confirmed_races() {
    let (repo, fetcher) = two_race_setup(Some(payouts_win8_140()), Some(payouts_quinella_1260()));
    let interactor = SettleInteractor::new(fetcher, repo);

    let report = interactor.settle_session(date()).await.unwrap();

    // 単勝 8: 1000/100*140=1400、馬連 6-8: 500/100*1260=6300。total_payout=7700。
    assert_eq!(report.settled_races, 2);
    assert_eq!(report.pending_races, 0);
    assert_eq!(report.total_bet, 1500);
    assert_eq!(report.total_payout, 7700);
    // balance = 10000 - 1500 + 7700 = 16200。
    assert_eq!(report.balance, 16200);
    // roi = 7700/1500*100 ≈ 513.3%
    assert!((report.roi.unwrap() - 513.333).abs() < 0.01);

    let written = interactor
        .repository
        .written
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    let (sess, settled) = written;
    assert!(sess.completed, "全レース確定で completed");
    assert_eq!(settled, vec![(1, 1400), (2, 6300)]);
}

#[tokio::test]
async fn pending_race_is_skipped_and_not_completed() {
    // 2R が未確定（payout 無し）。
    let (repo, fetcher) = two_race_setup(Some(payouts_win8_140()), None);
    let interactor = SettleInteractor::new(fetcher, repo);

    let report = interactor.settle_session(date()).await.unwrap();

    assert_eq!(report.settled_races, 1);
    assert_eq!(report.pending_races, 1);
    // 確定したのは 1R(単勝 1400)のみ。2R の payout は据え置き(0)。
    assert_eq!(report.total_payout, 1400);
    assert_eq!(report.balance, 10000 - 1500 + 1400);

    let written = interactor
        .repository
        .written
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    let (sess, settled) = written;
    assert!(!sess.completed, "未確定が残るため completed=false");
    // 書き込み対象は確定した 1R の bet のみ。
    assert_eq!(settled, vec![(1, 1400)]);
}

#[tokio::test]
async fn voided_race_refunds_all_bets_and_completes() {
    // 1R は確定、2R は開催中止・全馬取消（全額返還レース）。
    let (repo, fetcher) = two_race_setup(
        Some(payouts_win8_140()),
        Some(payouts_voided("2026-4-tokyo-1-2R")),
    );
    let interactor = SettleInteractor::new(fetcher, repo);

    let report = interactor.settle_session(date()).await.unwrap();

    // 1R 単勝 8: 1400。2R は全額返還で stake 500 をそのまま返戻。
    assert_eq!(report.settled_races, 1);
    assert_eq!(report.pending_races, 0);
    assert_eq!(report.voided_races, 1);
    assert_eq!(report.refunded_bets, 1);
    assert_eq!(report.total_payout, 1400 + 500);
    assert_eq!(report.balance, 10000 - 1500 + 1900);

    let written = interactor
        .repository
        .written
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    let (sess, settled) = written;
    assert!(
        sess.completed,
        "全額返還レースは pending を増やさず completed"
    );
    // 2R は payout=stake(500) で書き込まれる。
    assert_eq!(settled, vec![(1, 1400), (2, 500)]);
}

#[tokio::test]
async fn all_races_voided_refund_everything() {
    // 両レースとも開催中止・全馬取消。全買い目が stake 返戻され completed。
    let (repo, fetcher) = two_race_setup(
        Some(payouts_voided("2026-4-tokyo-1-1R")),
        Some(payouts_voided("2026-4-tokyo-1-2R")),
    );
    let interactor = SettleInteractor::new(fetcher, repo);

    let report = interactor.settle_session(date()).await.unwrap();

    assert_eq!(report.settled_races, 0);
    assert_eq!(report.pending_races, 0);
    assert_eq!(report.voided_races, 2);
    assert_eq!(report.refunded_bets, 2);
    // total_bet=1500 が全額返還 → total_payout=1500、balance=budget(10000)。
    assert_eq!(report.total_payout, 1500);
    assert_eq!(report.balance, 10000);
    assert_eq!(report.roi.unwrap(), 100.0);

    let written = interactor
        .repository
        .written
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    let (sess, settled) = written;
    assert!(sess.completed);
    assert_eq!(settled, vec![(1, 1000), (2, 500)]);
}

#[tokio::test]
async fn miss_pays_zero() {
    // 1R 単勝の的中が 3（買い目は 8）→ 不的中で 0。
    let mut p1 = RacePayouts::empty(RaceId::try_from("2026-4-tokyo-1-1R").unwrap());
    p1.insert("win", "3", 500);
    let (repo, fetcher) = two_race_setup(Some(p1), Some(payouts_quinella_1260()));
    let interactor = SettleInteractor::new(fetcher, repo);

    let report = interactor.settle_session(date()).await.unwrap();
    // 1R=0、2R=6300。
    assert_eq!(report.total_payout, 6300);
    let written = interactor
        .repository
        .written
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    assert_eq!(written.1, vec![(1, 0), (2, 6300)]);
}

#[tokio::test]
async fn idempotent_recompute_does_not_double_count() {
    // 既に payout 済みのセッション（前回 settle 済み）を再 settle しても同じ値になる。
    let r1 = "2026-4-tokyo-1-1R";
    let r2 = "2026-4-tokyo-1-2R";
    let bets = vec![
        (1i64, bet(r1, "win", "8", 1000, 1400)), // 前回 settle 済みの payout
        (2i64, bet(r2, "quinella", "6-8", 500, 6300)),
    ];
    let mut prior = session(10000, 1500);
    prior.total_payout = 7700;
    prior.balance = 16200;
    prior.completed = true;
    let repo = MockRepo {
        session: prior,
        bets,
        written: Mutex::new(None),
    };
    let mut payouts = HashMap::new();
    payouts.insert("202605040101".to_string(), payouts_win8_140());
    payouts.insert("202605040102".to_string(), payouts_quinella_1260());
    let interactor = SettleInteractor::new(FakeFetcher { payouts }, repo);

    let report = interactor.settle_session(date()).await.unwrap();
    // 二重加算されず前回と同値。
    assert_eq!(report.total_payout, 7700);
    assert_eq!(report.balance, 16200);
}

#[tokio::test]
async fn refunded_bet_returns_stake_and_is_counted() {
    use std::collections::HashSet;

    // 1R: 買い目は単勝 8 だが馬番 8 が出走取消 → stake 全額返戻。
    // 払戻表には別の的中(単勝 3)が載るが、取消馬を含む組番は配当照合より優先して返還。
    let mut p1 = RacePayouts::empty(RaceId::try_from("2026-4-tokyo-1-1R").unwrap());
    p1.insert("win", "3", 500);
    p1.set_scratched(HashSet::from([8]));
    // 2R: 馬連 6-8 は通常的中。
    let (repo, fetcher) = two_race_setup(Some(p1), Some(payouts_quinella_1260()));
    let interactor = SettleInteractor::new(fetcher, repo);

    let report = interactor.settle_session(date()).await.unwrap();

    assert_eq!(report.settled_races, 2);
    assert_eq!(report.refunded_bets, 1, "取消馬を含む 1 件が返還");
    // 1R 返還=1000（stake）、2R 的中=6300。total_payout=7300。
    assert_eq!(report.total_payout, 7300);
    // balance = 10000 - 1500 + 7300 = 15800（返還分は純収支 0）。
    assert_eq!(report.balance, 15800);

    let written = interactor
        .repository
        .written
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    // 返還買い目の payout は stake と一致（不的中の 0 と区別可能）。
    assert_eq!(written.1, vec![(1, 1000), (2, 6300)]);

    // 冪等性: 再 settle しても同値（返還も毎回ゼロから再計算）。
    let mut p1b = RacePayouts::empty(RaceId::try_from("2026-4-tokyo-1-1R").unwrap());
    p1b.insert("win", "3", 500);
    p1b.set_scratched(HashSet::from([8]));
    let bets = vec![
        (1i64, bet("2026-4-tokyo-1-1R", "win", "8", 1000, 1000)),
        (2i64, bet("2026-4-tokyo-1-2R", "quinella", "6-8", 500, 6300)),
    ];
    let mut prior = session(10000, 1500);
    prior.total_payout = 7300;
    prior.balance = 15800;
    let repo2 = MockRepo {
        session: prior,
        bets,
        written: Mutex::new(None),
    };
    let mut payouts = HashMap::new();
    payouts.insert("202605040101".to_string(), p1b);
    payouts.insert("202605040102".to_string(), payouts_quinella_1260());
    let interactor2 = SettleInteractor::new(FakeFetcher { payouts }, repo2);
    let report2 = interactor2.settle_session(date()).await.unwrap();
    assert_eq!(report2.total_payout, 7300);
    assert_eq!(report2.balance, 15800);
    assert_eq!(report2.refunded_bets, 1);
}

#[tokio::test]
async fn no_bets_returns_empty_report() {
    // 買い目無しはエラーではなく空 report（セッションの既存値を反映）。
    let (mut repo, fetcher) = two_race_setup(None, None);
    repo.bets.clear();
    let interactor = SettleInteractor::new(fetcher, repo);
    let report = interactor.settle_session(date()).await.unwrap();
    assert_eq!(report.settled_races, 0);
    assert_eq!(report.pending_races, 0);
    assert_eq!(report.total_payout, 0);
}

#[tokio::test]
async fn missing_session_returns_not_found() {
    struct NoSessionRepo;
    struct NoFetch;
    impl PayoutFetcher for NoFetch {
        fn fetch_race_payouts(&self, _: &str) -> Result<RacePayouts> {
            unimplemented!()
        }
    }
    impl PredictSessionRepository for NoSessionRepo {
        async fn find_predict_session(&self, _: NaiveDate) -> Result<Option<PredictSessionRecord>> {
            Ok(None)
        }
        async fn find_predict_bets_with_id(
            &self,
            _: NaiveDate,
        ) -> Result<Vec<(i64, PredictBetRecord)>> {
            unimplemented!()
        }
        async fn settle_predict_session(
            &self,
            _: &PredictSessionRecord,
            _: &[(i64, u64)],
        ) -> Result<()> {
            unimplemented!()
        }
        async fn find_predict_bets(&self, _: NaiveDate) -> Result<Vec<PredictBetRecord>> {
            unimplemented!()
        }
        async fn save_predict_session(&self, _: &PredictSessionRecord) -> Result<()> {
            unimplemented!()
        }
        async fn save_race_outcome(
            &self,
            _: NaiveDate,
            _: &RaceId,
            _: &[PredictBetRecord],
            _: DateTime<Utc>,
        ) -> Result<PredictSessionRecord> {
            unimplemented!()
        }
        async fn find_predict_race_conditions(
            &self,
            _: NaiveDate,
        ) -> Result<Vec<PredictRaceConditionRecord>> {
            unimplemented!()
        }
        async fn save_predict_race_condition(
            &self,
            _: NaiveDate,
            _: &PredictRaceConditionRecord,
            _: DateTime<Utc>,
        ) -> Result<()> {
            unimplemented!()
        }
    }

    let interactor = SettleInteractor::new(NoFetch, NoSessionRepo);
    let err = interactor.settle_session(date()).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}
