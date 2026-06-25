//! `find_jockey_recent_runs`（騎手直近フォーム #221 用）を Postgres で検証する:
//! `races.date < before` で当日より前の走のみ返し、date 降順・limit が効くこと。

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, JockeyFormRun, JockeyName, Race,
    RaceId, ResultStatus, Surface, Venue,
};
use paddock_use_case::repository::{RaceRepository, StatsRepository};
use rdb_gateway::PostgresRepository;

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// 指定騎手が騎乗した芝レースを 1 件作る（race_num=1 固定）。
fn race_with_jockey(
    race_id: &str,
    date: NaiveDate,
    jockey: &str,
    finish: u32,
    popularity: u32,
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
            horse_name: HorseName::try_from("テスト馬").unwrap(),
            horse_id: None,
            jockey: Some(JockeyName::try_from(jockey).unwrap()),
            trainer: None,
            time_seconds: None,
            margin: None,
            odds: None,
            horse_weight: None,
            weight_change: None,
            weight_carried: None,
            popularity: Some(popularity),
        }],
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn find_jockey_recent_runs_respects_cutoff_order_and_limit(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let jockey = JockeyName::try_from("田中 騎手").unwrap();

    // 田中騎手が 1/10, 2/10, 3/10 に騎乗。
    repo.save_race(&race_with_jockey(
        "jr-1",
        ymd(2026, 1, 10),
        "田中 騎手",
        3,
        4,
    ))
    .await
    .unwrap();
    repo.save_race(&race_with_jockey(
        "jr-2",
        ymd(2026, 2, 10),
        "田中 騎手",
        1,
        1,
    ))
    .await
    .unwrap();
    repo.save_race(&race_with_jockey(
        "jr-3",
        ymd(2026, 3, 10),
        "田中 騎手",
        5,
        7,
    ))
    .await
    .unwrap();

    // before=3/1: 3/10 は対象外、2/10・1/10 のみ。date 降順なので先頭は 2/10。
    let runs = repo
        .find_jockey_recent_runs(&jockey, ymd(2026, 3, 1), 5)
        .await
        .unwrap();
    assert_eq!(runs.len(), 2, "3/10 はカットオフ後で除外");
    assert_eq!(
        runs[0].finishing_position,
        Some(1),
        "直近走（2/10）の着順が取れている"
    );
    assert_eq!(runs[0].popularity, Some(1));
    assert_eq!(
        runs[1].finishing_position,
        Some(3),
        "2 番目（1/10）の着順が取れている"
    );

    // limit=1 なら直近 1 走のみ。
    let one = repo
        .find_jockey_recent_runs(&jockey, ymd(2026, 4, 1), 1)
        .await
        .unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].finishing_position, Some(5), "limit=1 で 3/10 のみ");

    // before が最初の出走以前なら空。
    let none = repo
        .find_jockey_recent_runs(&jockey, ymd(2026, 1, 10), 5)
        .await
        .unwrap();
    assert!(none.is_empty(), "当日含む以前は前走なし");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn jockey_recent_runs_batch_covers_all_jockeys(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);

    repo.save_race(&race_with_jockey(
        "jb-1",
        ymd(2026, 2, 1),
        "鈴木 騎手",
        1,
        2,
    ))
    .await
    .unwrap();
    repo.save_race(&race_with_jockey(
        "jb-2",
        ymd(2026, 2, 5),
        "佐藤 騎手",
        3,
        5,
    ))
    .await
    .unwrap();

    let jockeys = vec![
        JockeyName::try_from("鈴木 騎手").unwrap(),
        JockeyName::try_from("佐藤 騎手").unwrap(),
        // 近走なし騎手も map に含まれること
        JockeyName::try_from("未出走 騎手").unwrap(),
    ];
    let map = repo
        .jockey_recent_runs_batch(&jockeys, ymd(2026, 3, 1), 5)
        .await
        .unwrap();

    assert_eq!(map.len(), 3, "全騎手がキーとして存在する");
    let suzuki: &Vec<JockeyFormRun> = map
        .get(&JockeyName::try_from("鈴木 騎手").unwrap())
        .unwrap();
    assert_eq!(suzuki.len(), 1);
    assert_eq!(suzuki[0].finishing_position, Some(1));

    let sato: &Vec<JockeyFormRun> = map
        .get(&JockeyName::try_from("佐藤 騎手").unwrap())
        .unwrap();
    assert_eq!(sato.len(), 1);
    assert_eq!(sato[0].finishing_position, Some(3));

    let no_runs: &Vec<JockeyFormRun> = map
        .get(&JockeyName::try_from("未出走 騎手").unwrap())
        .unwrap();
    assert!(no_runs.is_empty(), "近走なし騎手は空 Vec");
}
