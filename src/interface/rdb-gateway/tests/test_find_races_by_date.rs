//! `find_races_by_date`（races ∪ race_cards）と `find_race_card` の date 往復を
//! Postgres（#[sqlx::test] の一時DB）で検証する。#26 の中核ロジックの回帰防止。

use chrono::{NaiveDate, NaiveTime};
use paddock_domain::{
    GateNum, HorseEntry, HorseName, HorseNum, Race, RaceCard, RaceId, Surface, TrackCondition,
    Venue,
};
use paddock_use_case::repository::{RaceCardRepository, RaceRepository};
use rdb_gateway::PostgresRepository;

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

/// 出馬表の発走時刻（#235 の往復検証・#391 の post_time 一括取得テストで共用）。
fn pt() -> NaiveTime {
    NaiveTime::from_hms_opt(15, 40, 0).unwrap()
}

/// 出馬表(race_cards)行。distance=1800 は race() の 2000 と区別して優先判定に使う。
fn card(race_id: &str, race_num: u32) -> RaceCard {
    RaceCard {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: d(),
        post_time: Some(pt()),
        venue: Venue::Nakayama,
        round: 3,
        day: 8,
        race_num,
        surface: Surface::Turf,
        distance: 1800,
        race_class: None,
        race_name: None,
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn returns_races_from_results(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn picks_up_race_cards_without_results(pool: sqlx::PgPool) {
    // #26 の核心: 成績がまだ無く出馬表だけでも開催日が拾えること。
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn dedupes_race_in_both_preferring_results(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn empty_when_no_data_for_date(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    repo.save_race_card(&card("2026-3-nakayama-8-1R", 1))
        .await
        .unwrap();
    // 別日には何も無い。
    let other = NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();
    assert!(repo.find_races_by_date(other).await.unwrap().is_empty());
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn find_race_card_round_trips_date(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let id = RaceId::try_from("2026-3-nakayama-8-1R").unwrap();
    repo.save_race_card(&card("2026-3-nakayama-8-1R", 1))
        .await
        .unwrap();

    let loaded = repo.find_race_card(&id).await.unwrap().unwrap();
    assert_eq!(loaded.date, d());
    assert_eq!(
        loaded.post_time,
        Some(pt()),
        "発走時刻が HH:MM で往復する（#235）"
    );
    assert_eq!(loaded.entries.len(), 1);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn find_race_card_post_time_none_round_trips(pool: sqlx::PgPool) {
    // PDF 経路・旧データ相当の post_time=None が NULL として保存され、None で読み戻る（#235）。
    let repo = PostgresRepository::new(pool);
    let id = RaceId::try_from("2026-3-nakayama-8-3R").unwrap();
    let mut c = card("2026-3-nakayama-8-3R", 3);
    c.post_time = None;
    repo.save_race_card(&c).await.unwrap();

    let loaded = repo.find_race_card(&id).await.unwrap().unwrap();
    assert_eq!(loaded.post_time, None, "post_time 未設定は None で往復する");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn find_post_times_by_date_maps_only_saved_post_times(pool: sqlx::PgPool) {
    // #391: 指定日の post_time を race_id → NaiveTime で一括取得。NULL 行は含まれない。
    let repo = PostgresRepository::new(pool);
    repo.save_race_card(&card("2026-3-nakayama-8-1R", 1))
        .await
        .unwrap();
    repo.save_race_card(&card("2026-3-nakayama-8-2R", 2))
        .await
        .unwrap();
    let mut no_post = card("2026-3-nakayama-8-3R", 3);
    no_post.post_time = None;
    repo.save_race_card(&no_post).await.unwrap();

    let map = repo.find_post_times_by_date(d()).await.unwrap();
    assert_eq!(map.len(), 2, "post_time が保存済みのレースだけ含まれる");
    let id = |s: &str| RaceId::try_from(s).unwrap();
    assert_eq!(map.get(&id("2026-3-nakayama-8-1R")), Some(&pt()));
    assert_eq!(map.get(&id("2026-3-nakayama-8-2R")), Some(&pt()));
    assert!(!map.contains_key(&id("2026-3-nakayama-8-3R")));

    // 別日は空マップ。
    let other = NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();
    assert!(
        repo.find_post_times_by_date(other)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn find_race_names_by_date_maps_only_saved_names(pool: sqlx::PgPool) {
    // #389: 指定日の race_name を race_id → String で一括取得。NULL 行（未設定）は含まれない。
    let repo = PostgresRepository::new(pool);
    let mut c1 = card("2026-3-nakayama-8-1R", 1);
    c1.race_name = Some("七夕賞".to_string());
    repo.save_race_card(&c1).await.unwrap();
    let mut c2 = card("2026-3-nakayama-8-2R", 2);
    c2.race_name = Some("響灘特別".to_string());
    repo.save_race_card(&c2).await.unwrap();
    // race_name 未設定（None）のレースはマップに含まれない。
    repo.save_race_card(&card("2026-3-nakayama-8-3R", 3))
        .await
        .unwrap();

    let map = repo.find_race_names_by_date(d()).await.unwrap();
    assert_eq!(map.len(), 2, "race_name が保存済みのレースだけ含まれる");
    let id = |s: &str| RaceId::try_from(s).unwrap();
    assert_eq!(
        map.get(&id("2026-3-nakayama-8-1R")).map(String::as_str),
        Some("七夕賞")
    );
    assert_eq!(
        map.get(&id("2026-3-nakayama-8-2R")).map(String::as_str),
        Some("響灘特別")
    );
    assert!(!map.contains_key(&id("2026-3-nakayama-8-3R")));

    // 別日は空マップ。
    let other = NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();
    assert!(
        repo.find_race_names_by_date(other)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn find_post_times_by_date_skips_unparsable_rows(pool: sqlx::PgPool) {
    // #391: HH:MM や race_id として解釈できない行は warn で縮退（無言破棄しない・全体は落とさない）。
    // save_race_card 経由では常に正規形になるため、破損データは生 SQL で直接作る。
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race_card(&card("2026-3-nakayama-8-1R", 1))
        .await
        .unwrap();
    sqlx::query(
        r#"
        INSERT INTO race_cards (race_id, venue, round, day, race_num, surface, distance, date, post_time)
        VALUES ('2026-3-nakayama-8-2R', 'nakayama', 3, 8, 2, 'turf', 1800, $1, 'invalid'),
               ('bad_race_id', 'nakayama', 3, 8, 3, 'turf', 1800, $1, '15:40')
        "#,
    )
    .bind(d().format("%Y-%m-%d").to_string())
    .execute(&pool)
    .await
    .unwrap();

    let map = repo.find_post_times_by_date(d()).await.unwrap();
    assert_eq!(
        map.len(),
        1,
        "解釈不能な post_time / race_id の行は除外され正常行は残る"
    );
    assert_eq!(
        map.get(&RaceId::try_from("2026-3-nakayama-8-1R").unwrap()),
        Some(&pt())
    );
}
