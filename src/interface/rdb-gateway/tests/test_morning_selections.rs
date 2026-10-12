//! `morning_selections`（#767: 朝の選定。レースごとに 1 行・先勝ち）を Postgres で往復検証する。
//! 先勝ち（2 回目の保存で上書きしない）は、再起動・`--once`・二重起動でも最初の選定を保つための要。

use chrono::NaiveDate;
use paddock_use_case::repository::{MorningSelection, MorningSelectionRepository};
use rdb_gateway::PostgresRepository;

fn date(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

fn selection(race_id: &str, day: u32, axis: u32) -> MorningSelection {
    MorningSelection {
        race_id: race_id.to_string(),
        date: date(day),
        axis,
        partners: vec![1, 3, 5, 9, 12],
        konsen_band: vec![],
        odds_fetched_at: "2026-10-10T00:31:05.123+00:00".to_string(),
        computed_at: "2026-10-10T01:00:00+00:00".to_string(),
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn selection_round_trips(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let mut s = selection("2026-4-tokyo-4-11R", 10, 7);
    s.konsen_band = vec![3, 5, 7, 9];
    repo.save_morning_selection(&s).await.unwrap();

    assert_eq!(
        repo.find_morning_selection("2026-4-tokyo-4-11R")
            .await
            .unwrap(),
        Some(s.clone())
    );
    assert_eq!(
        repo.find_morning_selections_by_date(date(10))
            .await
            .unwrap(),
        vec![s]
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn first_save_wins(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let first = selection("2026-4-tokyo-4-11R", 10, 7);
    repo.save_morning_selection(&first).await.unwrap();
    // 2 回目（別の軸）は何もしない
    repo.save_morning_selection(&selection("2026-4-tokyo-4-11R", 10, 2))
        .await
        .unwrap();

    let got = repo
        .find_morning_selection("2026-4-tokyo-4-11R")
        .await
        .unwrap();
    assert_eq!(got.map(|s| s.axis), Some(7));
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn by_date_filters_and_sorts(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    repo.save_morning_selection(&selection("2026-4-tokyo-4-12R", 10, 1))
        .await
        .unwrap();
    repo.save_morning_selection(&selection("2026-4-tokyo-4-1R", 10, 2))
        .await
        .unwrap();
    repo.save_morning_selection(&selection("2026-4-tokyo-5-1R", 11, 3))
        .await
        .unwrap();

    let ids: Vec<String> = repo
        .find_morning_selections_by_date(date(10))
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.race_id)
        .collect();
    assert_eq!(ids, vec!["2026-4-tokyo-4-12R", "2026-4-tokyo-4-1R"]);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn missing_is_empty(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    assert!(
        repo.find_morning_selections_by_date(date(10))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repo.find_morning_selection("2026-4-tokyo-4-11R")
            .await
            .unwrap(),
        None
    );
}
