//! `predict_sessions` / `predict_bets` の永続化を実 SQLite（temp ファイル）で往復検証する。
//! オッズ未整備のためライブセッションでは買い目を発生させられないので、賭けを伴う
//! payout/bets の保存・復元はこの結合テストで担保する。

use chrono::{NaiveDate, Utc};
use paddock_domain::{RaceId, TrackCondition};
use paddock_use_case::repository::{
    PredictBetRecord, PredictRaceConditionRecord, PredictSessionRecord, Repository,
};
use rdb_gateway::{SqliteRepository, pool};

async fn fresh_repo() -> (SqliteRepository, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let p = pool::connect(&url).await.expect("connect");
    pool::migrate(&p).await.expect("migrate");
    (SqliteRepository::new(p), dir)
}

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, 19).unwrap()
}

fn bet(combo: &str, code: &str, stake: u64, payout: u64, ev: f64) -> PredictBetRecord {
    PredictBetRecord {
        race_id: RaceId::try_from("2026-3-nakayama-8-1R").unwrap(),
        bet_type: combo.to_string(),
        combination: code.to_string(),
        stake,
        payout,
        ev,
    }
}

#[tokio::test]
async fn session_header_round_trips() {
    let (repo, _dir) = fresh_repo().await;
    let now = Utc::now();
    let session = PredictSessionRecord {
        date: date(),
        budget: 10_000,
        balance: 10_000,
        total_bet: 0,
        total_payout: 0,
        completed: false,
        created_at: now,
        updated_at: now,
    };

    assert!(repo.find_predict_session(date()).await.unwrap().is_none());
    repo.save_predict_session(&session).await.unwrap();

    let loaded = repo.find_predict_session(date()).await.unwrap().unwrap();
    assert_eq!(loaded.date, date());
    assert_eq!(loaded.budget, 10_000);
    assert_eq!(loaded.balance, 10_000);
    assert!(!loaded.completed);
}

#[tokio::test]
async fn save_race_outcome_updates_balance_and_persists_bets() {
    let (repo, _dir) = fresh_repo().await;
    let now = Utc::now();
    let mut session = PredictSessionRecord {
        date: date(),
        budget: 10_000,
        balance: 10_000,
        total_bet: 0,
        total_payout: 0,
        completed: false,
        created_at: now,
        updated_at: now,
    };
    repo.save_predict_session(&session).await.unwrap();

    // 単勝3 ¥1000（外れ）＋ 馬連1-5 ¥500（払戻¥2500）
    // 残高: 10000 - 1000 - 500 + 2500 = 11000
    session.balance = 11_000;
    session.total_bet = 1_500;
    session.total_payout = 2_500;
    session.updated_at = Utc::now();
    let race_id = RaceId::try_from("2026-3-nakayama-8-1R").unwrap();
    let bets = vec![
        bet("win", "3", 1_000, 0, 1.5),
        bet("quinella", "1-5", 500, 2_500, 1.8),
    ];
    repo.save_race_outcome(&session, &race_id, &bets)
        .await
        .unwrap();

    let loaded = repo.find_predict_session(date()).await.unwrap().unwrap();
    assert_eq!(loaded.balance, 11_000);
    assert_eq!(loaded.total_bet, 1_500);
    assert_eq!(loaded.total_payout, 2_500);

    let saved = repo.find_predict_bets(date()).await.unwrap();
    assert_eq!(saved.len(), 2);
    // bet_id 昇順 = 挿入順
    assert_eq!(saved[0].bet_type, "win");
    assert_eq!(saved[0].combination, "3");
    assert_eq!(saved[0].stake, 1_000);
    assert_eq!(saved[0].payout, 0);
    assert_eq!(saved[1].combination, "1-5");
    assert_eq!(saved[1].payout, 2_500);
    assert_eq!(saved[1].race_id.value(), "2026-3-nakayama-8-1R");
}

#[tokio::test]
async fn completed_flag_and_multi_race_append() {
    let (repo, _dir) = fresh_repo().await;
    let now = Utc::now();
    let mut session = PredictSessionRecord {
        date: date(),
        budget: 10_000,
        balance: 10_000,
        total_bet: 0,
        total_payout: 0,
        completed: false,
        created_at: now,
        updated_at: now,
    };
    repo.save_predict_session(&session).await.unwrap();

    let r1 = RaceId::try_from("2026-3-nakayama-8-1R").unwrap();
    session.balance = 9_000;
    session.total_bet = 1_000;
    repo.save_race_outcome(&session, &r1, &[bet("win", "3", 1_000, 0, 1.5)])
        .await
        .unwrap();

    let r2 = RaceId::try_from("2026-3-nakayama-8-2R").unwrap();
    let mut b2 = bet("place", "7", 800, 1_200, 1.3);
    b2.race_id = r2.clone();
    session.balance = 9_400;
    session.total_bet = 1_800;
    session.total_payout = 1_200;
    repo.save_race_outcome(&session, &r2, &[b2]).await.unwrap();

    // 2 レース分の買い目が蓄積される
    let saved = repo.find_predict_bets(date()).await.unwrap();
    assert_eq!(saved.len(), 2);

    // 完了マーク
    session.completed = true;
    session.updated_at = Utc::now();
    repo.save_predict_session(&session).await.unwrap();
    let loaded = repo.find_predict_session(date()).await.unwrap().unwrap();
    assert!(loaded.completed);
    assert_eq!(loaded.total_bet, 1_800);
    assert_eq!(loaded.total_payout, 1_200);
}

/// セッションヘッダを先に作る（predict_race_conditions.session_date の FK 充足）。
async fn seed_session(repo: &SqliteRepository) {
    let now = Utc::now();
    repo.save_predict_session(&PredictSessionRecord {
        date: date(),
        budget: 10_000,
        balance: 10_000,
        total_bet: 0,
        total_payout: 0,
        completed: false,
        created_at: now,
        updated_at: now,
    })
    .await
    .unwrap();
}

fn cond(race: &str, tc: Option<TrackCondition>) -> PredictRaceConditionRecord {
    PredictRaceConditionRecord {
        race_id: RaceId::try_from(race).unwrap(),
        track_condition: tc,
    }
}

#[tokio::test]
async fn race_condition_round_trips_value_and_unknown() {
    let (repo, _dir) = fresh_repo().await;
    seed_session(&repo).await;
    assert!(
        repo.find_predict_race_conditions(date())
            .await
            .unwrap()
            .is_empty()
    );

    let now = Utc::now();
    // 値あり（稍重）と「不明として記録」(None) を別レースで保存する。
    repo.save_predict_race_condition(
        date(),
        &cond("2026-3-nakayama-8-1R", Some(TrackCondition::Good)),
        now,
    )
    .await
    .unwrap();
    repo.save_predict_race_condition(date(), &cond("2026-3-nakayama-8-2R", None), now)
        .await
        .unwrap();

    let loaded = repo.find_predict_race_conditions(date()).await.unwrap();
    // race_id 昇順で返る。
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].race_id.value(), "2026-3-nakayama-8-1R");
    assert_eq!(loaded[0].track_condition, Some(TrackCondition::Good));
    assert_eq!(loaded[1].race_id.value(), "2026-3-nakayama-8-2R");
    assert_eq!(loaded[1].track_condition, None);
}

#[tokio::test]
async fn race_condition_upsert_overwrites_same_race() {
    let (repo, _dir) = fresh_repo().await;
    seed_session(&repo).await;
    let now = Utc::now();

    repo.save_predict_race_condition(
        date(),
        &cond("2026-3-nakayama-8-1R", Some(TrackCondition::Firm)),
        now,
    )
    .await
    .unwrap();
    // 同一レースを再入力（重→上書き）。行は増えず値だけ更新される。
    repo.save_predict_race_condition(
        date(),
        &cond("2026-3-nakayama-8-1R", Some(TrackCondition::Yielding)),
        Utc::now(),
    )
    .await
    .unwrap();

    let loaded = repo.find_predict_race_conditions(date()).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].track_condition, Some(TrackCondition::Yielding));
}
