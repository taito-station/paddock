//! `race_odds` の保存(save_race_odds)→読み出し(find_race_odds)を実 SQLite で往復検証する。
//! 単勝・複勝の復元と、backtest 用の `as_of`（`date(fetched_at) <= d`）境界を担保する。

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use paddock_domain::{HorseNum, RaceId};
use paddock_use_case::repository::{OddsRow, RaceOddsRecord, Repository};
use rdb_gateway::{SqliteRepository, pool};

async fn fresh_repo() -> (SqliteRepository, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let p = pool::connect(&url).await.expect("connect");
    pool::migrate(&p).await.expect("migrate");
    (SqliteRepository::new(p), dir)
}

fn race_id() -> RaceId {
    RaceId::try_from("2026-3-nakayama-8-1R").unwrap()
}

fn horse(n: u32) -> HorseNum {
    HorseNum::try_from(n).unwrap()
}

fn fetched_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 19, 10, 0, 0).unwrap()
}

/// 単勝 2 頭 + 複勝 2 頭を 1 レコードで保存する。
async fn save_sample(repo: &SqliteRepository) {
    let record = RaceOddsRecord {
        race_id: race_id(),
        fetched_at: fetched_at(),
        rows: vec![
            OddsRow {
                bet_type: "win".to_string(),
                combination_key: "1".to_string(),
                odds: 3.5,
                odds_high: None,
                popularity: Some(2),
            },
            OddsRow {
                bet_type: "win".to_string(),
                combination_key: "2".to_string(),
                odds: 7.1,
                odds_high: None,
                popularity: Some(5),
            },
            OddsRow {
                bet_type: "place".to_string(),
                combination_key: "1".to_string(),
                odds: 1.5,
                odds_high: Some(2.0),
                popularity: Some(2),
            },
            OddsRow {
                bet_type: "place".to_string(),
                combination_key: "2".to_string(),
                odds: 2.2,
                odds_high: Some(3.4),
                popularity: Some(5),
            },
        ],
    };
    repo.save_race_odds(&record).await.unwrap();
}

#[tokio::test]
async fn round_trips_win_and_place() {
    let (repo, _dir) = fresh_repo().await;
    save_sample(&repo).await;

    let odds = repo
        .find_race_odds(&race_id(), None)
        .await
        .unwrap()
        .expect("保存済みオッズが読めること");

    // 単勝: そのままの値で復元。
    assert_eq!(odds.win.len(), 2);
    assert!((odds.win.get(&horse(1)).unwrap().value() - 3.5).abs() < 1e-9);
    assert!((odds.win.get(&horse(2)).unwrap().value() - 7.1).abs() < 1e-9);

    // 複勝: odds=下限, odds_high=上限 として幅で復元。
    assert_eq!(odds.place.len(), 2);
    let p1 = odds.place.get(&horse(1)).unwrap();
    assert!((p1.low.value() - 1.5).abs() < 1e-9);
    assert!((p1.high.value() - 2.0).abs() < 1e-9);
}

#[tokio::test]
async fn returns_none_when_absent() {
    let (repo, _dir) = fresh_repo().await;
    let other = RaceId::try_from("2026-3-nakayama-8-9R").unwrap();
    assert!(repo.find_race_odds(&other, None).await.unwrap().is_none());
}

#[tokio::test]
async fn as_of_filters_on_fetched_at_date() {
    let (repo, _dir) = fresh_repo().await;
    save_sample(&repo).await; // fetched_at = 2026-04-19

    // 当日以降の as_of は対象（date(fetched_at) <= as_of）。
    let same_day = NaiveDate::from_ymd_opt(2026, 4, 19).unwrap();
    assert!(
        repo.find_race_odds(&race_id(), Some(same_day))
            .await
            .unwrap()
            .is_some(),
        "fetched_at と同日は当時オッズとして参照できる"
    );

    // 前日の as_of では未来のスナップショット扱いで除外され None。
    let day_before = NaiveDate::from_ymd_opt(2026, 4, 18).unwrap();
    assert!(
        repo.find_race_odds(&race_id(), Some(day_before))
            .await
            .unwrap()
            .is_none(),
        "as_of より後に取得されたオッズはリーク防止で除外される"
    );
}
