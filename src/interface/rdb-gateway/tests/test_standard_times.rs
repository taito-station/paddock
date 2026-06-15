//! `standard_times`（前走タイム相対速度シグナル #76 用）を実 SQLite で検証する:
//! (surface, distance) 別に完走タイムを平均し、最小標本数未満のバケツを除外、`date < before`
//! で as-of リークを防ぐこと。

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, Race, RaceId, ResultStatus,
    Surface, TimeSeconds, Venue,
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

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// `times` の各要素を 1 頭ぶんの完走結果（タイムあり）にした 1 レースを作る。
fn race_with_times(
    race_id: &str,
    date: NaiveDate,
    surface: Surface,
    distance: u32,
    times: &[f64],
) -> Race {
    let results = times
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            let n = (i + 1) as u32;
            HorseResult {
                finishing_position: Some(FinishingPosition::try_from(n).unwrap()),
                status: ResultStatus::Finished,
                gate_num: GateNum::try_from(1u32).unwrap(),
                horse_num: HorseNum::try_from(n).unwrap(),
                horse_name: HorseName::try_from(format!("ウマ{n}").as_str()).unwrap(),
                horse_id: None,
                jockey: None,
                trainer: None,
                time_seconds: Some(TimeSeconds::try_from(t).unwrap()),
                margin: None,
                odds: None,
                horse_weight: None,
                weight_change: None,
                weight_carried: None,
                popularity: None,
            }
        })
        .collect();
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date,
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 1,
        surface,
        distance,
        track_condition: None,
        weather: None,
        results,
    }
}

#[tokio::test]
async fn standard_times_averages_and_drops_thin_buckets() {
    let (repo, _dir) = fresh_repo().await;
    // 芝2000: 5 サンプル（>= 閾値）→ 平均 100.0 を採用。
    repo.save_race(&race_with_times(
        "turf-2000",
        ymd(2026, 1, 10),
        Surface::Turf,
        2000,
        &[98.0, 99.0, 100.0, 101.0, 102.0],
    ))
    .await
    .unwrap();
    // ダ1200: 2 サンプル（< 閾値）→ 除外。
    repo.save_race(&race_with_times(
        "dirt-1200",
        ymd(2026, 1, 11),
        Surface::Dirt,
        1200,
        &[72.0, 73.0],
    ))
    .await
    .unwrap();

    let st = repo.standard_times(ymd(2026, 2, 1)).await.unwrap();
    assert_eq!(st.get(Surface::Turf, 2000), Some(100.0), "5 件の平均");
    assert_eq!(st.get(Surface::Dirt, 1200), None, "閾値未満は除外");
}

#[tokio::test]
async fn standard_times_respects_as_of_cutoff() {
    let (repo, _dir) = fresh_repo().await;
    // before=2/1 より後（3/10）のレースは集計から除外される（リーク防止）。
    repo.save_race(&race_with_times(
        "turf-1600-future",
        ymd(2026, 3, 10),
        Surface::Turf,
        1600,
        &[95.0, 96.0, 97.0, 98.0, 99.0],
    ))
    .await
    .unwrap();

    let st = repo.standard_times(ymd(2026, 2, 1)).await.unwrap();
    assert_eq!(
        st.get(Surface::Turf, 1600),
        None,
        "cutoff 以降のレースは標準タイムに含めない"
    );
}
