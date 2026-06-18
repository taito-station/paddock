//! netkeiba 近走を `horses`/`horse_past_runs` に分離した #59 の検証（Postgres）:
//! - `upsert_horse_history` が horses/horse_past_runs に入り、results/races を汚さない
//! - 集計（horse_stats）は pdf のみで二重計上しない
//! - `find_recent_runs` は pdf と netkeiba を UNION し、同一実レースは pdf 優先で 1 件に dedup

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseId, HorseName, HorseNum, HorseResult, Race, RaceId,
    ResultStatus, Surface, Venue,
};
use paddock_use_case::HorsePastRun;
use paddock_use_case::repository::{HorseHistoryRepository, RaceRepository, StatsRepository};
use rdb_gateway::PostgresRepository;

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn past_run(nk_id: &str, horse: &str, date: NaiveDate, race_num: u32, finish: u32) -> HorsePastRun {
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
        horse_name: HorseName::try_from(horse).unwrap(),
        jockey: None,
        time_seconds: None,
        margin: None,
        odds: None,
        horse_weight: None,
        weight_change: None,
        weight_carried: None,
        popularity: None,
    }
}

/// pdf 確定成績 1 レース（東京 3 回 2 日 11R）を作る。canonical race_id は netkeiba 側と揃える。
fn pdf_race(race_id: &str, date: NaiveDate, race_num: u32, horse: &str, finish: u32) -> Race {
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

async fn count(repo: &PostgresRepository, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let row: (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .fetch_one(&repo.pool)
        .await
        .unwrap();
    row.0
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn upsert_horse_history_lands_in_separate_tables(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let horse_id = HorseId::try_from("2019104567".to_string()).unwrap();
    // 12 桁 netkeiba id（東京=05）。canonical: 2026-3-tokyo-2-11R / 2026-3-tokyo-2-12R。
    let runs = vec![
        past_run("202605030211", "ウマZ", ymd(2026, 4, 1), 11, 1),
        past_run("202605030212", "ウマZ", ymd(2026, 3, 1), 12, 3),
    ];
    repo.upsert_horse_history(&horse_id, &runs).await.unwrap();

    assert_eq!(count(&repo, "horses").await, 1, "horses に 1 頭");
    assert_eq!(count(&repo, "horse_past_runs").await, 2, "近走 2 走");
    assert_eq!(count(&repo, "results").await, 0, "results は汚さない");
    assert_eq!(count(&repo, "races").await, 0, "races は汚さない");

    // netkeiba 12 桁が canonical paddock race_id に変換されて保存される（dedup が依存する不変条件）。
    let rid: (String,) = sqlx::query_as(
        "SELECT race_id FROM horse_past_runs WHERE netkeiba_race_id = '202605030211'",
    )
    .fetch_one(&repo.pool)
    .await
    .unwrap();
    assert_eq!(rid.0, "2026-3-tokyo-2-11R");

    // 冪等性: 同じ取得を再実行しても増えない（ON CONFLICT 上書き）。
    repo.upsert_horse_history(&horse_id, &runs).await.unwrap();
    assert_eq!(
        count(&repo, "horse_past_runs").await,
        2,
        "再取り込みで重複しない"
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn upsert_skips_unconvertible_run_and_saves_rest(pool: sqlx::PgPool) {
    // canonical race_id へ変換できない走（例: 場コード 44 = 地方/JRA 外）が混ざっても、その
    // 1 走だけ skip して残りは保存し、バッチ全体を止めないことを固定する回帰テスト（#103 の耐性）。
    // ※開催回 7 以上は #111 で変換可能になったため skip 例には使わない（下の別テストで保存を確認）。
    let repo = PostgresRepository::new(pool);
    let horse_id = HorseId::try_from("2019104567".to_string()).unwrap();
    let runs = vec![
        past_run("202605030211", "ウマZ", ymd(2026, 4, 1), 11, 1),
        // 5〜6 桁目の場コード 44 = 地方（JRA 外）→ paddock_race_id_from_netkeiba が弾く。
        // 変換は netkeiba_race_id 文字列のみを見るため、past_run の他フィールドとは無関係。
        past_run("202644030211", "ウマZ", ymd(2026, 4, 2), 11, 2),
        past_run("202605030212", "ウマZ", ymd(2026, 3, 1), 12, 3),
    ];
    let saved = repo.upsert_horse_history(&horse_id, &runs).await.unwrap();

    assert_eq!(saved, 2, "変換できた 2 走のみ保存（非JRA の 1 走は skip）");
    assert_eq!(count(&repo, "horse_past_runs").await, 2);
    assert_eq!(count(&repo, "horses").await, 1, "horses マスタは更新される");

    let skipped: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM horse_past_runs WHERE netkeiba_race_id = '202644030211'",
    )
    .fetch_one(&repo.pool)
    .await
    .unwrap();
    assert_eq!(skipped.0, 0, "変換できない非JRA の走は保存されない");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn upsert_saves_round_over_six_run(pool: sqlx::PgPool) {
    // #111: netkeiba は一部 JRA レースに開催回 7 以上を採番する（例 2024 京都 7 回）。
    // これらも canonical 変換でき、skip されず保存されることを固定する。
    let repo = PostgresRepository::new(pool);
    let horse_id = HorseId::try_from("2019104567".to_string()).unwrap();
    let saved = repo
        .upsert_horse_history(
            &horse_id,
            &[past_run("202408070706", "ウマZ", ymd(2024, 6, 1), 6, 1)],
        )
        .await
        .unwrap();

    assert_eq!(saved, 1, "開催回 7 の走も保存される（#111）");
    let rid: (String,) = sqlx::query_as(
        "SELECT race_id FROM horse_past_runs WHERE netkeiba_race_id = '202408070706'",
    )
    .fetch_one(&repo.pool)
    .await
    .unwrap();
    assert_eq!(rid.0, "2024-7-kyoto-7-6R");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn horse_stats_counts_pdf_only_not_netkeiba(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // pdf で 1 戦 1 勝。
    repo.save_race(&pdf_race(
        "2026-3-tokyo-2-11R",
        ymd(2026, 4, 1),
        11,
        "ウマZ",
        1,
    ))
    .await
    .unwrap();
    // netkeiba 近走（同一実レース + 別レース）を投入しても horse_stats は変わらない。
    let horse_id = HorseId::try_from("2019104567".to_string()).unwrap();
    repo.upsert_horse_history(
        &horse_id,
        &[
            past_run("202605030211", "ウマZ", ymd(2026, 4, 1), 11, 1),
            past_run("202605030212", "ウマZ", ymd(2026, 3, 1), 12, 3),
        ],
    )
    .await
    .unwrap();

    let stats = repo
        .horse_stats(&HorseName::try_from("ウマZ").unwrap(), None)
        .await
        .unwrap();
    assert_eq!(stats.overall.starts, 1, "pdf の 1 戦のみ（二重計上しない）");
    assert_eq!(stats.overall.wins, 1);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn find_recent_runs_unions_and_dedups_preferring_pdf(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // 11R は pdf(1着) と netkeiba(7着) の両方＝同一実レース。12R は netkeiba のみ。
    repo.save_race(&pdf_race(
        "2026-3-tokyo-2-11R",
        ymd(2026, 4, 1),
        11,
        "ウマZ",
        1,
    ))
    .await
    .unwrap();
    let horse_id = HorseId::try_from("2019104567".to_string()).unwrap();
    repo.upsert_horse_history(
        &horse_id,
        &[
            past_run("202605030211", "ウマZ", ymd(2026, 4, 1), 11, 7),
            past_run("202605030212", "ウマZ", ymd(2026, 3, 1), 12, 3),
        ],
    )
    .await
    .unwrap();

    let runs = repo
        .find_recent_runs(&HorseName::try_from("ウマZ").unwrap(), ymd(2026, 5, 1), 5)
        .await
        .unwrap();

    assert_eq!(runs.len(), 2, "11R は 1 件に dedup、12R は単独 → 計 2");
    // date 降順: 先頭は 4/1 の 11R。pdf 優先なので着順は 1（netkeiba の 7 ではない）。
    assert_eq!(runs[0].date, ymd(2026, 4, 1));
    assert_eq!(
        runs[0].result.finishing_position.map(|p| p.value()),
        Some(1),
        "同一実レースは pdf を優先（netkeiba の 7 着で上書きされない）"
    );
    // 2 件目は netkeiba のみの 3/1 12R。
    assert_eq!(runs[1].date, ymd(2026, 3, 1));
    assert_eq!(
        runs[1].result.finishing_position.map(|p| p.value()),
        Some(3)
    );
}
