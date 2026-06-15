//! `find_races_by_date`（races ∪ race_cards）と `find_race_card` の date 往復を
//! 実 SQLite（temp ファイル）で検証する。#26 の中核ロジックの回帰防止。

use chrono::NaiveDate;
use paddock_domain::{
    GateNum, HorseEntry, HorseName, HorseNum, Race, RaceCard, RaceId, Surface, TrackCondition,
    Venue,
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

fn d() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, 19).unwrap()
}

/// 成績(races)行。track_condition / distance を race_cards 側と変えて優先判定に使う。
fn race(race_id: &str, race_num: u32) -> Race {
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: d(),
        venue: Venue::Nakayama,
        round: 3,
        day: 8,
        race_num,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: Some(TrackCondition::Firm),
        weather: None,
        results: Vec::new(),
    }
}

/// 出馬表(race_cards)行。distance=1800 は race() の 2000 と区別して優先判定に使う。
fn card(race_id: &str, race_num: u32) -> RaceCard {
    RaceCard {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: d(),
        venue: Venue::Nakayama,
        round: 3,
        day: 8,
        race_num,
        surface: Surface::Turf,
        distance: 1800,
        entries: vec![HorseEntry {
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(1u32).unwrap(),
            horse_name: HorseName::try_from("テストウマ").unwrap(),
            jockey: None,
            trainer: None,
            weight_carried: None,
        }],
    }
}

#[tokio::test]
async fn returns_races_from_results() {
    let (repo, _dir) = fresh_repo().await;
    repo.save_race(&race("2026-3-nakayama-8-2R", 2))
        .await
        .unwrap();
    repo.save_race(&race("2026-3-nakayama-8-1R", 1))
        .await
        .unwrap();

    let found = repo.find_races_by_date(d()).await.unwrap();
    assert_eq!(found.len(), 2);
    // race_num 昇順
    assert_eq!(found[0].race_num, 1);
    assert_eq!(found[1].race_num, 2);
}

#[tokio::test]
async fn picks_up_race_cards_without_results() {
    // #26 の核心: 成績がまだ無く出馬表だけでも開催日が拾えること。
    let (repo, _dir) = fresh_repo().await;
    repo.save_race_card(&card("2026-3-nakayama-8-1R", 1))
        .await
        .unwrap();
    repo.save_race_card(&card("2026-3-nakayama-8-2R", 2))
        .await
        .unwrap();

    let found = repo.find_races_by_date(d()).await.unwrap();
    assert_eq!(found.len(), 2, "出馬表のみでも 2 レース拾えること");
    assert!(
        found.iter().all(|r| r.track_condition.is_none()),
        "race_cards 由来は track_condition が None"
    );
    assert!(found.iter().all(|r| r.results.is_empty()));
}

#[tokio::test]
async fn dedupes_race_in_both_preferring_results() {
    let (repo, _dir) = fresh_repo().await;
    // 1R は成績＋出馬表の両方、2R は出馬表のみ。
    repo.save_race(&race("2026-3-nakayama-8-1R", 1))
        .await
        .unwrap();
    repo.save_race_card(&card("2026-3-nakayama-8-1R", 1))
        .await
        .unwrap();
    repo.save_race_card(&card("2026-3-nakayama-8-2R", 2))
        .await
        .unwrap();

    let found = repo.find_races_by_date(d()).await.unwrap();
    assert_eq!(found.len(), 2, "重複 race_id は 1 件に集約される");

    let r1 = found.iter().find(|r| r.race_num == 1).unwrap();
    // 両方に存在する race は成績(races)側が優先される。
    assert!(r1.track_condition.is_some(), "成績側を優先");
    assert_eq!(r1.distance, 2000, "成績側 distance(2000) が採用される");
}

#[tokio::test]
async fn empty_when_no_data_for_date() {
    let (repo, _dir) = fresh_repo().await;
    repo.save_race_card(&card("2026-3-nakayama-8-1R", 1))
        .await
        .unwrap();
    // 別日には何も無い。
    let other = NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();
    assert!(repo.find_races_by_date(other).await.unwrap().is_empty());
}

#[tokio::test]
async fn find_race_card_round_trips_date() {
    let (repo, _dir) = fresh_repo().await;
    let id = RaceId::try_from("2026-3-nakayama-8-1R").unwrap();
    repo.save_race_card(&card("2026-3-nakayama-8-1R", 1))
        .await
        .unwrap();

    let loaded = repo.find_race_card(&id).await.unwrap().unwrap();
    assert_eq!(loaded.date, d());
    assert_eq!(loaded.entries.len(), 1);
}
