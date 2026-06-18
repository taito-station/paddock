//! `find_recent_runs`（前走フォーム #31 用）を Postgres で検証する:
//! `races.date < before` で前走のみ返し、date 降順・limit が効くこと。

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, Race, RaceId, ResultStatus,
    Surface, Venue,
};
use paddock_use_case::repository::{RaceRepository, StatsRepository};
use rdb_gateway::PostgresRepository;

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// 指定馬が出走した芝レースを 1 件作る（着順・人気・体重変化を指定）。
fn race_run(
    race_id: &str,
    date: NaiveDate,
    horse: &str,
    finish: u32,
    popularity: u32,
    weight_change: i32,
) -> Race {
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date,
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 1,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        weather: None,
        results: vec![HorseResult {
            finishing_position: Some(FinishingPosition::try_from(finish).unwrap()),
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
            horse_weight: Some(480),
            weight_change: Some(weight_change),
            weight_carried: None,
            popularity: Some(popularity),
        }],
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn find_recent_runs_respects_cutoff_order_and_limit(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // ウマX が 1/10, 2/10, 3/10 に出走。
    repo.save_race(&race_run("r-1", ymd(2026, 1, 10), "ウマX", 3, 4, 2))
        .await
        .unwrap();
    repo.save_race(&race_run("r-2", ymd(2026, 2, 10), "ウマX", 1, 1, -2))
        .await
        .unwrap();
    repo.save_race(&race_run("r-3", ymd(2026, 3, 10), "ウマX", 5, 7, 6))
        .await
        .unwrap();

    let name = HorseName::try_from("ウマX").unwrap();

    // before=3/1: 3/10 は対象外、2/10・1/10 のみ。date 降順なので先頭は 2/10。
    let runs = repo
        .find_recent_runs(&name, ymd(2026, 3, 1), 5)
        .await
        .unwrap();
    assert_eq!(runs.len(), 2, "3/10 はカットオフ後で除外");
    assert_eq!(runs[0].date, ymd(2026, 2, 10), "date 降順で直近が先頭");
    assert_eq!(runs[1].date, ymd(2026, 1, 10));
    // 前走の (surface, distance) を運んでいる（#76 標準タイム突合用）。
    assert_eq!(runs[0].surface, Surface::Turf);
    assert_eq!(runs[0].distance, 2000);
    // 前走(2/10)の中身が取れている
    assert_eq!(
        runs[0].result.finishing_position.map(|p| p.value()),
        Some(1)
    );
    assert_eq!(runs[0].result.popularity, Some(1));
    assert_eq!(runs[0].result.weight_change, Some(-2));

    // limit=1 なら直近 1 走のみ。
    let one = repo
        .find_recent_runs(&name, ymd(2026, 4, 1), 1)
        .await
        .unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].date, ymd(2026, 3, 10));

    // before が最初の出走以前なら空（前走なし）。
    let none = repo
        .find_recent_runs(&name, ymd(2026, 1, 10), 5)
        .await
        .unwrap();
    assert!(none.is_empty(), "当日含む以前は前走なし");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn find_recent_runs_is_deterministic_on_same_date_ties(pool: sqlx::PgPool) {
    // 同一馬の同一日に 2 走（pdf と netkeiba の二重登録を模す。race_id 違い）。
    // race_id 降順タイブレークにより LIMIT 1 の選択が決定的になること。
    let repo = PostgresRepository::new(pool);
    repo.save_race(&race_run("aaa-low", ymd(2026, 2, 10), "ウマY", 8, 9, 0))
        .await
        .unwrap();
    repo.save_race(&race_run("zzz-high", ymd(2026, 2, 10), "ウマY", 1, 1, 0))
        .await
        .unwrap();

    let name = HorseName::try_from("ウマY").unwrap();
    // race_id 降順 → "zzz-high" が先頭で安定。
    for _ in 0..3 {
        let runs = repo
            .find_recent_runs(&name, ymd(2026, 3, 1), 1)
            .await
            .unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].result.finishing_position.map(|p| p.value()),
            Some(1),
            "race_id 降順タイブレークで zzz-high(1着) が決定的に選ばれる"
        );
    }
}
