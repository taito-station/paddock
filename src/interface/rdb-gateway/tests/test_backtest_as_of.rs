//! バックテスト基盤 (#30) の中核を Postgres で検証する:
//! - `horse_stats` の as-of 日付カットオフ（`races.date < D`）がリークを防ぐこと
//! - `find_finished_races_between` が期間内の確定レースを results 付きで返し、`from > to` で空になること

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

/// 1 頭が 1 着になった芝 2000m の確定レースを作る。
fn race_with_winner(race_id: &str, date: NaiveDate, race_num: u32, horse: &str) -> Race {
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date,
        venue: Venue::Nakayama,
        round: 3,
        day: 8,
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
            odds: Some(3.0),
            horse_weight: None,
            weight_change: None,
            weight_carried: None,
            popularity: None,
        }],
    }
}

fn turf_starts(row: &paddock_use_case::repository::HorseStatsRow) -> (u32, u32) {
    let g = row
        .by_surface
        .iter()
        .find(|g| g.label == "芝")
        .expect("芝 group");
    (g.starts, g.wins)
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn horse_stats_as_of_excludes_same_day_and_future(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // 同一馬が 1/1 と 2/1 に各 1 勝。
    repo.save_race(&race_with_winner(
        "2026-1-1-1R",
        ymd(2026, 1, 1),
        1,
        "ウマX",
    ))
    .await
    .unwrap();
    repo.save_race(&race_with_winner(
        "2026-2-1-1R",
        ymd(2026, 2, 1),
        1,
        "ウマX",
    ))
    .await
    .unwrap();

    let name = HorseName::try_from("ウマX").unwrap();

    // 全期間: 2 戦 2 勝
    let all = repo.horse_stats(&name, None).await.unwrap();
    assert_eq!(turf_starts(&all), (2, 2));

    // as_of = 2/1（当日）: races.date < 2026-02-01 のみ → 1/1 の 1 戦だけ（当日・未来を除外）
    let as_of_feb1 = repo
        .horse_stats(&name, Some(ymd(2026, 2, 1)))
        .await
        .unwrap();
    assert_eq!(
        turf_starts(&as_of_feb1),
        (1, 1),
        "D 当日(2/1)と以降をリークさせない"
    );

    // as_of = 1/1（最初のレース当日）: それより前は無し → 0 戦
    let as_of_jan1 = repo
        .horse_stats(&name, Some(ymd(2026, 1, 1)))
        .await
        .unwrap();
    assert_eq!(turf_starts(&as_of_jan1), (0, 0), "当日も除外で 0 戦");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn find_finished_races_between_returns_results_in_range(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    repo.save_race(&race_with_winner(
        "2026-1-1-1R",
        ymd(2026, 1, 1),
        1,
        "ウマA",
    ))
    .await
    .unwrap();
    repo.save_race(&race_with_winner(
        "2026-2-1-1R",
        ymd(2026, 2, 1),
        1,
        "ウマB",
    ))
    .await
    .unwrap();
    repo.save_race(&race_with_winner(
        "2026-3-1-1R",
        ymd(2026, 3, 1),
        1,
        "ウマC",
    ))
    .await
    .unwrap();

    let found = repo
        .find_finished_races_between(ymd(2026, 1, 1), ymd(2026, 2, 28))
        .await
        .unwrap();
    assert_eq!(found.len(), 2, "1月・2月の 2 レース");
    // date 昇順
    assert_eq!(found[0].date, ymd(2026, 1, 1));
    assert_eq!(found[1].date, ymd(2026, 2, 1));
    // results が付いている
    assert_eq!(found[0].results.len(), 1);
    assert_eq!(
        found[0].results[0].finishing_position.map(|p| p.value()),
        Some(1)
    );
    assert_eq!(found[0].results[0].odds, Some(3.0));
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn find_finished_races_between_empty_when_from_after_to(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    repo.save_race(&race_with_winner(
        "2026-2-1-1R",
        ymd(2026, 2, 1),
        1,
        "ウマB",
    ))
    .await
    .unwrap();

    let found = repo
        .find_finished_races_between(ymd(2026, 3, 2), ymd(2026, 3, 1))
        .await
        .unwrap();
    assert!(found.is_empty(), "from > to は空集合");
}
