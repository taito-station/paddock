//! #60: pdf 成績行の horse_id backfill を実 SQLite で検証する。
//! - 馬名が horses にちょうど 1 件一致 → horse_id が埋まる
//! - 同名別馬（horses に 2 件）/ horses に無い名前 → NULL 据え置き
//! - 冪等性（2 回目は 0 行）・既存 horse_id は上書きしない

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, Race, RaceId, ResultStatus,
    Surface, Venue,
};
use paddock_use_case::repository::Repository;
use rdb_gateway::{SqliteRepository, pool};

async fn fresh_repo() -> (SqliteRepository, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let p = pool::connect(&url).await.expect("connect");
    pool::migrate(&p).await.expect("migrate");
    (SqliteRepository::new(p), dir)
}

/// pdf 成績 1 頭分のレースを作る（horse_id は None＝pdf 経路相当）。
fn pdf_race(race_id: &str, race_num: u32, horse: &str) -> Race {
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
        venue: Venue::Tokyo,
        round: 3,
        day: 2,
        race_num,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        weather: None,
        results: vec![HorseResult {
            finishing_position: Some(FinishingPosition::try_from(1u32).unwrap()),
            status: ResultStatus::Finished,
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(1u32).unwrap(),
            horse_name: HorseName::try_from(horse).unwrap(),
            horse_id: None,
            jockey: None,
            trainer: None,
            time_seconds: None,
            margin: None,
            odds: None,
            horse_weight: None,
            weight_change: None,
            weight_carried: None,
            popularity: None,
        }],
    }
}

async fn insert_horse(repo: &SqliteRepository, horse_id: &str, name: &str) {
    sqlx::query("INSERT INTO horses (horse_id, horse_name) VALUES (?, ?)")
        .bind(horse_id)
        .bind(name)
        .execute(&repo.pool)
        .await
        .unwrap();
}

async fn horse_id_of(repo: &SqliteRepository, race_id: &str) -> Option<String> {
    let row: (Option<String>,) = sqlx::query_as("SELECT horse_id FROM results WHERE race_id = ?")
        .bind(race_id)
        .fetch_one(&repo.pool)
        .await
        .unwrap();
    row.0
}

#[tokio::test]
async fn backfills_only_unique_name_matches() {
    let (repo, _dir) = fresh_repo().await;
    // ユニーク一致 / 同名別馬（2 件）/ horses に無い名前 の 3 ケース。
    insert_horse(&repo, "2019104567", "ウマユニーク").await;
    insert_horse(&repo, "2018100001", "ウマ被り").await;
    insert_horse(&repo, "2018100002", "ウマ被り").await;

    repo.save_race(&pdf_race("2026-3-tokyo-2-1R", 1, "ウマユニーク"))
        .await
        .unwrap();
    repo.save_race(&pdf_race("2026-3-tokyo-2-2R", 2, "ウマ被り"))
        .await
        .unwrap();
    repo.save_race(&pdf_race("2026-3-tokyo-2-3R", 3, "ウマ未登録"))
        .await
        .unwrap();

    let filled = repo.backfill_results_horse_ids().await.unwrap();
    assert_eq!(filled, 1, "一意一致の 1 行だけ埋まる");

    assert_eq!(
        horse_id_of(&repo, "2026-3-tokyo-2-1R").await.as_deref(),
        Some("2019104567"),
        "ユニーク一致は埋まる"
    );
    assert_eq!(
        horse_id_of(&repo, "2026-3-tokyo-2-2R").await,
        None,
        "同名別馬は NULL 据え置き"
    );
    assert_eq!(
        horse_id_of(&repo, "2026-3-tokyo-2-3R").await,
        None,
        "horses に無い名前は NULL"
    );
}

#[tokio::test]
async fn fills_multiple_rows_and_returns_total_count() {
    let (repo, _dir) = fresh_repo().await;
    insert_horse(&repo, "2019100001", "ウマA").await;
    insert_horse(&repo, "2019100002", "ウマB").await;
    repo.save_race(&pdf_race("2026-3-tokyo-2-1R", 1, "ウマA"))
        .await
        .unwrap();
    repo.save_race(&pdf_race("2026-3-tokyo-2-2R", 2, "ウマB"))
        .await
        .unwrap();

    let filled = repo.backfill_results_horse_ids().await.unwrap();
    assert_eq!(filled, 2, "一意一致が複数 → rows_affected は合計");
    assert_eq!(
        horse_id_of(&repo, "2026-3-tokyo-2-1R").await.as_deref(),
        Some("2019100001")
    );
    assert_eq!(
        horse_id_of(&repo, "2026-3-tokyo-2-2R").await.as_deref(),
        Some("2019100002")
    );
}

#[tokio::test]
async fn idempotent_and_preserves_existing() {
    let (repo, _dir) = fresh_repo().await;
    insert_horse(&repo, "2019104567", "ウマユニーク").await;
    repo.save_race(&pdf_race("2026-3-tokyo-2-1R", 1, "ウマユニーク"))
        .await
        .unwrap();

    assert_eq!(repo.backfill_results_horse_ids().await.unwrap(), 1);
    // 2 回目は対象なし（horse_id IS NULL のみ対象）。
    assert_eq!(
        repo.backfill_results_horse_ids().await.unwrap(),
        0,
        "冪等: 2 回目は 0 行"
    );

    // 別 horse_id へ手動変更後に再実行しても上書きしない。
    sqlx::query("UPDATE results SET horse_id = ? WHERE race_id = ?")
        .bind("9999999999")
        .bind("2026-3-tokyo-2-1R")
        .execute(&repo.pool)
        .await
        .unwrap();
    assert_eq!(repo.backfill_results_horse_ids().await.unwrap(), 0);
    assert_eq!(
        horse_id_of(&repo, "2026-3-tokyo-2-1R").await.as_deref(),
        Some("9999999999"),
        "既存 horse_id は上書きされない"
    );
}
