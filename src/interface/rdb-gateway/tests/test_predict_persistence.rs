//! `predict_sessions` / `predict_bets` の永続化を Postgres（#[sqlx::test] の一時DB）で往復検証する。
//! オッズ未整備のためライブセッションでは買い目を発生させられないので、賭けを伴う
//! payout/bets の保存・復元はこの結合テストで担保する。

use chrono::{NaiveDate, Utc};
use paddock_domain::{RaceId, TrackCondition};
use paddock_use_case::repository::{
    PredictBetRecord, PredictRaceConditionRecord, PredictSessionRecord, PredictSessionRepository,
};
use rdb_gateway::PostgresRepository;

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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn session_header_round_trips(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn save_race_outcome_updates_balance_and_persists_bets(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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
    repo.save_predict_session(&session).await.unwrap();

    // 単勝3 ¥1000（外れ）＋ 馬連1-5 ¥500（払戻¥2500）
    // 残高: 10000 - 1000 - 500 + 2500 = 11000（残高計算は save_race_outcome が FOR UPDATE 下で行う）
    let race_id = RaceId::try_from("2026-3-nakayama-8-1R").unwrap();
    let bets = vec![
        bet("win", "3", 1_000, 0, 1.5),
        bet("quinella", "1-5", 500, 2_500, 1.8),
    ];
    let updated = repo
        .save_race_outcome(date(), &race_id, &bets, Utc::now())
        .await
        .unwrap();
    // 返り値の更新後セッションが残高計算済みであること。
    assert_eq!(updated.balance, 11_000);
    assert_eq!(updated.total_bet, 1_500);
    assert_eq!(updated.total_payout, 2_500);

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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn completed_flag_and_multi_race_append(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

    // R1: 単勝3 ¥1000（外れ）→ 残高 9000 / total_bet 1000
    let r1 = RaceId::try_from("2026-3-nakayama-8-1R").unwrap();
    let after_r1 = repo
        .save_race_outcome(date(), &r1, &[bet("win", "3", 1_000, 0, 1.5)], Utc::now())
        .await
        .unwrap();
    assert_eq!(after_r1.balance, 9_000);
    assert_eq!(after_r1.total_bet, 1_000);

    // R2: 複勝7 ¥800（払戻¥1200）→ 残高 9400 / total_bet 1800 / total_payout 1200
    let r2 = RaceId::try_from("2026-3-nakayama-8-2R").unwrap();
    let mut b2 = bet("place", "7", 800, 1_200, 1.3);
    b2.race_id = r2.clone();
    let after_r2 = repo
        .save_race_outcome(date(), &r2, &[b2], Utc::now())
        .await
        .unwrap();
    assert_eq!(after_r2.balance, 9_400);
    assert_eq!(after_r2.total_bet, 1_800);
    assert_eq!(after_r2.total_payout, 1_200);

    // 2 レース分の買い目が蓄積される
    let saved = repo.find_predict_bets(date()).await.unwrap();
    assert_eq!(saved.len(), 2);

    // 完了マーク（save_race_outcome の返り値をベースにヘッダのみ更新）
    session = after_r2;
    session.completed = true;
    session.updated_at = Utc::now();
    repo.save_predict_session(&session).await.unwrap();
    let loaded = repo.find_predict_session(date()).await.unwrap().unwrap();
    assert!(loaded.completed);
    assert_eq!(loaded.total_bet, 1_800);
    assert_eq!(loaded.total_payout, 1_200);
}

/// #469 の回帰: 二重記録ガード・残高ガード・未作成ガードが FOR UPDATE トランザクション内で
/// 効くこと（check-then-act の分離をなくしたこと）を DB 往復で固定する。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn save_race_outcome_enforces_guards_atomically(pool: sqlx::PgPool) {
    use paddock_use_case::Error as UcError;

    let repo = PostgresRepository::new(pool);

    // 未作成セッションへの記録は NotFound（HTTP 404）。
    let r1 = RaceId::try_from("2026-3-nakayama-8-1R").unwrap();
    let err = repo
        .save_race_outcome(date(), &r1, &[bet("win", "3", 1_000, 0, 1.5)], Utc::now())
        .await
        .unwrap_err();
    assert!(matches!(err, UcError::NotFound(_)), "got {err:?}");

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

    // 残高超過は InvalidArgument（HTTP 400）で状態不変。
    let err = repo
        .save_race_outcome(date(), &r1, &[bet("win", "3", 15_000, 0, 1.5)], Utc::now())
        .await
        .unwrap_err();
    assert!(matches!(err, UcError::InvalidArgument(_)), "got {err:?}");
    let loaded = repo.find_predict_session(date()).await.unwrap().unwrap();
    assert_eq!(loaded.balance, 10_000, "残高超過拒否後はセッション不変");
    assert!(repo.find_predict_bets(date()).await.unwrap().is_empty());

    // 正常記録（残高 10000 - 1000 = 9000）。
    repo.save_race_outcome(date(), &r1, &[bet("win", "3", 1_000, 0, 1.5)], Utc::now())
        .await
        .unwrap();

    // 同一レースへ買い目ありで再記録すると Conflict（HTTP 409・買い目重複＋残高二重適用を防ぐ）。
    let err = repo
        .save_race_outcome(date(), &r1, &[bet("win", "3", 500, 0, 1.5)], Utc::now())
        .await
        .unwrap_err();
    assert!(matches!(err, UcError::Conflict(_)), "got {err:?}");

    // Conflict 後も残高・買い目は 1 回分のまま（二重適用されていない）。
    let loaded = repo.find_predict_session(date()).await.unwrap().unwrap();
    assert_eq!(loaded.balance, 9_000);
    assert_eq!(loaded.total_bet, 1_000);
    assert_eq!(repo.find_predict_bets(date()).await.unwrap().len(), 1);

    // 買い目なしの再送はスキップの冪等再送として許容（Conflict にしない）。
    repo.save_race_outcome(date(), &r1, &[], Utc::now())
        .await
        .unwrap();
    // 購入済みレースに空 bets を送っても skip 痕跡は付かない（購入済みが正・見送りではない）。
    assert!(
        repo.find_predict_race_skips(date())
            .await
            .unwrap()
            .is_empty()
    );
}

/// #481: 空 bets の見送りを skip 表に per-race 保存し、往復・冪等・購入への遷移で正しく振る舞う。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn skip_round_trips_and_clears_on_purchase(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    seed_session(&repo).await;
    let r1 = RaceId::try_from("2026-1-nakayama-1-R1").unwrap();
    let r2 = RaceId::try_from("2026-1-nakayama-1-R2").unwrap();

    assert!(
        repo.find_predict_race_skips(date())
            .await
            .unwrap()
            .is_empty()
    );

    // r1 を見送り。残高は不変・bets は増えない・skip 表に載る。
    let s = repo
        .save_race_outcome(date(), &r1, &[], Utc::now())
        .await
        .unwrap();
    assert_eq!(s.balance, 10_000);
    assert!(repo.find_predict_bets(date()).await.unwrap().is_empty());
    let skips = repo.find_predict_race_skips(date()).await.unwrap();
    assert_eq!(skips, vec![r1.clone()]);

    // 空 bets の再送は冪等（重複しない）。
    repo.save_race_outcome(date(), &r1, &[], Utc::now())
        .await
        .unwrap();
    assert_eq!(
        repo.find_predict_race_skips(date()).await.unwrap(),
        vec![r1.clone()]
    );

    // r1 を後から買い目ありで記録すると skip 痕跡は消え、bets 側に現れる。
    repo.save_race_outcome(date(), &r1, &[bet("win", "3", 1_000, 0, 1.5)], Utc::now())
        .await
        .unwrap();
    assert!(
        repo.find_predict_race_skips(date())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(repo.find_predict_bets(date()).await.unwrap().len(), 1);

    // 別レース r2 の見送りは独立して載る（race_id 昇順で返る）。
    repo.save_race_outcome(date(), &r2, &[], Utc::now())
        .await
        .unwrap();
    assert_eq!(
        repo.find_predict_race_skips(date()).await.unwrap(),
        vec![r2.clone()]
    );
}

/// #469 の核心: 同一レースへの並行記録を 2 本同時に走らせても、FOR UPDATE で直列化され
/// 片方のみ成功・もう片方は Conflict になる（買い目重複＋残高二重適用が起きない）。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn concurrent_record_same_race_serializes_and_rejects_duplicate(pool: sqlx::PgPool) {
    use paddock_use_case::Error as UcError;

    let repo = PostgresRepository::new(pool.clone());
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

    let r1 = RaceId::try_from("2026-3-nakayama-8-1R").unwrap();
    // 2 つの独立したリポジトリ（別コネクション）から同一レースへ同時 POST を再現する。
    let repo_a = PostgresRepository::new(pool.clone());
    let repo_b = PostgresRepository::new(pool.clone());
    let r_a = r1.clone();
    let r_b = r1.clone();
    let t_a = tokio::spawn(async move {
        repo_a
            .save_race_outcome(date(), &r_a, &[bet("win", "3", 1_000, 0, 1.5)], Utc::now())
            .await
    });
    let t_b = tokio::spawn(async move {
        repo_b
            .save_race_outcome(date(), &r_b, &[bet("win", "3", 1_000, 0, 1.5)], Utc::now())
            .await
    });
    let res_a = t_a.await.unwrap();
    let res_b = t_b.await.unwrap();

    // ちょうど 1 本が成功し、もう 1 本は Conflict（二重記録拒否）。
    let ok_count = [&res_a, &res_b].iter().filter(|r| r.is_ok()).count();
    assert_eq!(
        ok_count, 1,
        "並行記録は 1 本のみ成功: a={res_a:?} b={res_b:?}"
    );
    let conflict_count = [&res_a, &res_b]
        .iter()
        .filter(|r| matches!(r, Err(UcError::Conflict(_))))
        .count();
    assert_eq!(
        conflict_count, 1,
        "もう 1 本は Conflict: a={res_a:?} b={res_b:?}"
    );

    // 残高は 1 回分（10000 - 1000 = 9000）のみ・買い目も 1 件のみ（二重適用なし）。
    let loaded = repo.find_predict_session(date()).await.unwrap().unwrap();
    assert_eq!(loaded.balance, 9_000);
    assert_eq!(loaded.total_bet, 1_000);
    assert_eq!(repo.find_predict_bets(date()).await.unwrap().len(), 1);
}

/// セッションヘッダを先に作る（predict_race_conditions.session_date の FK 充足）。
async fn seed_session(repo: &PostgresRepository) {
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn race_condition_round_trips_value_and_unknown(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn race_condition_upsert_overwrites_same_race(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    seed_session(&repo).await;
    let now = Utc::now();

    repo.save_predict_race_condition(
        date(),
        &cond("2026-3-nakayama-8-1R", Some(TrackCondition::Firm)),
        now,
    )
    .await
    .unwrap();
    // 同一レースを再入力（良→重 で上書き）。行は増えず値だけ更新される。
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn race_condition_save_requires_existing_session_fk(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // セッションヘッダ未作成のまま保存すると FK 制約（session_date → predict_sessions）違反で
    // エラーになる。FK 制約宣言が有効（Postgres は FK を常時強制）であることの回帰検知。
    let res = repo
        .save_predict_race_condition(
            date(),
            &cond("2026-3-nakayama-8-1R", Some(TrackCondition::Firm)),
            Utc::now(),
        )
        .await;
    assert!(
        res.is_err(),
        "セッション無しでの保存は FK 制約で失敗するはず"
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn race_condition_upsert_preserves_created_at_and_advances_updated_at(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    seed_session(&repo).await;

    let t1 = Utc::now();
    let t2 = t1 + chrono::Duration::seconds(60);
    let race = "2026-3-nakayama-8-1R";
    repo.save_predict_race_condition(date(), &cond(race, Some(TrackCondition::Firm)), t1)
        .await
        .unwrap();
    repo.save_predict_race_condition(date(), &cond(race, Some(TrackCondition::Yielding)), t2)
        .await
        .unwrap();

    // 上書き時、created_at は初回値を保持し updated_at のみ更新されることを raw SQL で確認する。
    // PredictRaceConditionRecord は時刻を保持しないため find 経由では検証できない。
    let (created_at, updated_at): (String, String) = sqlx::query_as(
        "SELECT created_at, updated_at FROM predict_race_conditions \
         WHERE session_date = $1 AND race_id = $2",
    )
    .bind(date().format("%Y-%m-%d").to_string())
    .bind(race)
    .fetch_one(&repo.pool)
    .await
    .unwrap();

    assert_eq!(created_at, t1.to_rfc3339(), "created_at は初回値を保持する");
    assert_eq!(
        updated_at,
        t2.to_rfc3339(),
        "updated_at は最新の保存時刻に更新される"
    );
}
