//! `standard_times`（前走タイム相対速度シグナル #76 用）を Postgres で検証する:
//! (surface, distance) 別に完走タイムを平均し、最小標本数未満のバケツを除外、`date < before`
//! で as-of リークを防ぐこと。

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseId, HorseName, HorseNum, HorseResult, Race, RaceId,
    ResultStatus, Surface, TimeSeconds, Venue,
};
use paddock_use_case::HorsePastRun;
use paddock_use_case::repository::{HorseHistoryRepository, RaceRepository, StatsRepository};
use rdb_gateway::PostgresRepository;

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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn standard_times_averages_and_drops_thin_buckets(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn standard_times_respects_as_of_cutoff(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

/// netkeiba 近走 1 走（芝2000, タイムあり）。`upsert_horse_history` 経由で horse_past_runs に入る。
fn nk_run(nk_id: &str, date: NaiveDate, race_num: u32, time: f64) -> HorsePastRun {
    HorsePastRun {
        netkeiba_race_id: nk_id.to_string(),
        date,
        venue: Venue::Tokyo,
        round: 1,
        day: 1,
        race_num,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        finishing_position: Some(FinishingPosition::try_from(1u32).unwrap()),
        status: ResultStatus::Finished,
        gate_num: GateNum::try_from(1u32).unwrap(),
        horse_num: HorseNum::try_from(1u32).unwrap(),
        horse_name: HorseName::try_from("ウマN").unwrap(),
        jockey: None,
        time_seconds: Some(TimeSeconds::try_from(time).unwrap()),
        margin: None,
        odds: None,
        horse_weight: None,
        weight_change: None,
        weight_carried: None,
        popularity: None,
        race_name: None,
        corner_positions: None,
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn standard_times_aggregates_pdf_and_netkeiba_sources(pool: sqlx::PgPool) {
    // 標準タイムは results(pdf) と horse_past_runs(netkeiba) の UNION で集計する。
    // どちらか単独では閾値(5)未満だが、両ソースを合算すると到達するケースで
    // netkeiba UNION 分岐の実行と cross-source 集計を固定する。
    let repo = PostgresRepository::new(pool);
    // pdf 1 件（芝2000, 100.0 秒）。
    repo.save_race(&race_with_times(
        "turf-2000-pdf",
        ymd(2026, 1, 10),
        Surface::Turf,
        2000,
        &[100.0],
    ))
    .await
    .unwrap();
    // netkeiba 近走 4 走（芝2000, いずれも 100.0 秒）。
    let horse_id = HorseId::try_from("2019104567".to_string()).unwrap();
    let runs = vec![
        nk_run("202605010101", ymd(2026, 1, 1), 1, 100.0),
        nk_run("202605010102", ymd(2026, 1, 2), 2, 100.0),
        nk_run("202605010103", ymd(2026, 1, 3), 3, 100.0),
        nk_run("202605010104", ymd(2026, 1, 4), 4, 100.0),
    ];
    repo.upsert_horse_history(&horse_id, &runs).await.unwrap();

    // pdf 1 + netkeiba 4 = 5 サンプルで閾値到達（単独ソースでは未満）→ UNION 合算を検証。
    let st = repo.standard_times(ymd(2026, 2, 1)).await.unwrap();
    assert_eq!(
        st.get(Surface::Turf, 2000),
        Some(100.0),
        "pdf と netkeiba が同一 (surface,distance) バケツに合算され閾値到達"
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn standard_times_excludes_zero_seconds(pool: sqlx::PgPool) {
    // TimeSeconds は 0.0 を許容するため、0 秒の異常行が平均・標本数に混ざらないことを固定する
    // （SQL の `time_seconds > 0` ガード）。0 秒 1 件 + 100 秒 5 件 → 0 秒を除いた平均 100.0、
    // 母数も 5 件として閾値判定される（0 秒を数えると 6 件・平均は下振れする）。
    let repo = PostgresRepository::new(pool);
    repo.save_race(&race_with_times(
        "turf-2000-zero",
        ymd(2026, 1, 10),
        Surface::Turf,
        2000,
        &[0.0, 100.0, 100.0, 100.0, 100.0, 100.0],
    ))
    .await
    .unwrap();

    let st = repo.standard_times(ymd(2026, 2, 1)).await.unwrap();
    assert_eq!(
        st.get(Surface::Turf, 2000),
        Some(100.0),
        "0 秒の異常行は平均・標本数から除外される"
    );
}
