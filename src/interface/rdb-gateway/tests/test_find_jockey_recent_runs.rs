//! `find_jockey_recent_runs`（騎手直近フォーム #221 用）を Postgres で検証する:
//! `races.date < before` で当日より前の走のみ返し、date 降順・limit が効くこと。

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseId, HorseName, HorseNum, HorseResult, JockeyFormRun,
    JockeyName, Race, RaceId, ResultStatus, Surface, Venue,
};
use paddock_use_case::HorsePastRun;
use paddock_use_case::repository::{HorseHistoryRepository, RaceRepository, StatsRepository};
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

/// 同一実レース（東京 3 回 2 日 11R）が pdf(`results`) と netkeiba(`horse_past_runs`) の双方に
/// 存在する場合に、`(date, venue, race_num)` 単位で pdf 優先 dedup されて 1 件になることを検証する
/// （find_recent_runs.rs の `find_recent_runs_unions_and_dedups_preferring_pdf` の騎手版）。
fn pdf_race_with_jockey(
    race_id: &str,
    date: NaiveDate,
    race_num: u32,
    jockey: &str,
    finish: u32,
) -> Race {
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date,
        venue: Venue::Tokyo,
        round: 3,
        day: 2,
        race_num,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        weather: None,
        results: vec![HorseResult {
            finishing_position: Some(FinishingPosition::try_from(finish).unwrap()),
            status: ResultStatus::Finished,
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(1u32).unwrap(),
            horse_name: HorseName::try_from("ウマZ").unwrap(),
            horse_id: None,
            jockey: Some(JockeyName::try_from(jockey).unwrap()),
            trainer: None,
            time_seconds: None,
            margin: None,
            odds: None,
            horse_weight: None,
            weight_change: None,
            weight_carried: None,
            popularity: Some(1),
        }],
    }
}

fn past_run_with_jockey(
    nk_id: &str,
    date: NaiveDate,
    race_num: u32,
    jockey: &str,
    finish: u32,
) -> HorsePastRun {
    HorsePastRun {
        netkeiba_race_id: nk_id.to_string(),
        date,
        venue: Venue::Tokyo,
        round: 3,
        day: 2,
        race_num,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        finishing_position: Some(FinishingPosition::try_from(finish).unwrap()),
        status: ResultStatus::Finished,
        gate_num: GateNum::try_from(1u32).unwrap(),
        horse_num: HorseNum::try_from(1u32).unwrap(),
        horse_name: HorseName::try_from("ウマZ").unwrap(),
        jockey: Some(JockeyName::try_from(jockey).unwrap()),
        time_seconds: None,
        margin: None,
        odds: None,
        horse_weight: None,
        weight_change: None,
        weight_carried: None,
        popularity: Some(5),
        race_name: None,
        corner_positions: None,
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn find_jockey_recent_runs_unions_and_dedups_preferring_pdf(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let jockey_name = "山田 騎手";
    // 11R は pdf(1着) と netkeiba(7着) の両方＝同一実レース。12R は netkeiba のみ（3着）。
    repo.save_race(&pdf_race_with_jockey(
        "2026-3-tokyo-2-11R",
        ymd(2026, 4, 1),
        11,
        jockey_name,
        1,
    ))
    .await
    .unwrap();
    let horse_id = HorseId::try_from("2019104567".to_string()).unwrap();
    repo.upsert_horse_history(
        &horse_id,
        &[
            past_run_with_jockey("202605030211", ymd(2026, 4, 1), 11, jockey_name, 7),
            past_run_with_jockey("202605030212", ymd(2026, 3, 1), 12, jockey_name, 3),
        ],
    )
    .await
    .unwrap();

    let jockey = JockeyName::try_from(jockey_name).unwrap();
    let runs = repo
        .find_jockey_recent_runs(&jockey, ymd(2026, 5, 1), 5)
        .await
        .unwrap();

    assert_eq!(runs.len(), 2, "11R は 1 件に dedup、12R は単独 → 計 2");
    // date 降順: 先頭は 4/1 の 11R。pdf 優先なので着順は 1（netkeiba の 7 ではない）。
    assert_eq!(
        runs[0].finishing_position,
        Some(1),
        "同一実レースは pdf を優先（netkeiba の 7 着で上書きされない）"
    );
    // 2 件目は netkeiba のみの 3/1 12R（3着）。
    assert_eq!(runs[1].finishing_position, Some(3));

    // batch 版でも同じ dedup 挙動になること。
    let map = repo
        .jockey_recent_runs_batch(std::slice::from_ref(&jockey), ymd(2026, 5, 1), 5)
        .await
        .unwrap();
    let batch_runs = map.get(&jockey).unwrap();
    assert_eq!(batch_runs.len(), 2, "batch も dedup して計 2");
    assert_eq!(
        batch_runs[0].finishing_position,
        Some(1),
        "batch も pdf 優先"
    );
}
