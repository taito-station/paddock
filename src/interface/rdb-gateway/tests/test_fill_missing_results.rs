//! 足りない馬番の結果行だけを INSERT する口（#742）を Postgres で検証する:
//! - 既存行は全列そのまま（修正済みの馬名・`status='scratched'` でも上書きしない）
//! - races 行（全列）と他のレースの行は変わらない
//! - 新しい行は `source='pdf'`・`gate_num` と馬名は出馬表から・`horse_id` と `margin` は NULL
//! - 出馬表に無い馬番は入れない。2 回目は 0 行（冪等）。races 行が無ければエラーで何も書かない

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseEntry, HorseName, HorseNum, HorseResult, JockeyName, Race,
    RaceCard, RaceId, ResultStatus, Surface, Venue,
};
use paddock_use_case::netkeiba_scraper::ResultRow;
use paddock_use_case::repository::{RaceCardRepository, RaceRepository};
use rdb_gateway::{ExistingResultRow, PostgresRepository};

const RID: &str = "2026-2-niigata-6-7R";
const OTHER: &str = "2026-2-niigata-6-8R";

fn d() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 8, 9).unwrap()
}

fn entry(n: u32) -> HorseEntry {
    HorseEntry {
        gate_num: GateNum::try_from(n.div_ceil(2)).unwrap(),
        horse_num: HorseNum::try_from(n).unwrap(),
        horse_name: HorseName::try_from(format!("ウマ{n}")).unwrap(),
        jockey: None,
        trainer: None,
        weight_carried: None,
    }
}

fn card(race_id: &str, nums: &[u32]) -> RaceCard {
    RaceCard {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: d(),
        post_time: None,
        venue: Venue::Niigata,
        round: 2,
        day: 6,
        race_num: 7,
        surface: Surface::Turf,
        distance: 1600,
        race_class: None,
        race_name: None,
        entries: nums.iter().map(|&n| entry(n)).collect(),
    }
}

fn pdf_result(n: u32) -> HorseResult {
    HorseResult {
        finishing_position: Some(FinishingPosition::try_from(n).unwrap()),
        status: ResultStatus::Finished,
        gate_num: GateNum::try_from(n.div_ceil(2)).unwrap(),
        horse_num: HorseNum::try_from(n).unwrap(),
        horse_name: HorseName::try_from(format!("ウマ{n}")).unwrap(),
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
    }
}

fn race(race_id: &str, results: Vec<HorseResult>) -> Race {
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: d(),
        venue: Venue::Niigata,
        round: 2,
        day: 6,
        race_num: 7,
        surface: Surface::Turf,
        distance: 1600,
        track_condition: None,
        weather: None,
        results,
    }
}

fn page_row(n: u32, pos: u32) -> ResultRow {
    ResultRow {
        horse_num: HorseNum::try_from(n).unwrap(),
        finishing_position: Some(FinishingPosition::try_from(pos).unwrap()),
        status: ResultStatus::Finished,
        jockey: Some(JockeyName::try_from("騎手A").unwrap()),
        trainer: None,
        time_seconds: None,
        odds: Some(12.3),
        horse_weight: Some(480),
        weight_change: Some(-2),
        weight_carried: Some(57.0),
        popularity: Some(5),
    }
}

/// 表の全列を主キー順に連結した文字列（前後比較用）。
async fn snapshot(repo: &PostgresRepository, sql: &'static str) -> String {
    let rows: Vec<(String,)> = sqlx::query_as(sql).fetch_all(&repo.pool).await.unwrap();
    rows.into_iter().map(|r| r.0).collect::<Vec<_>>().join("\n")
}

const EXISTING_SQL: &str = "SELECT row_to_json(r)::text FROM results r WHERE race_id = '2026-2-niigata-6-7R' AND horse_num = 1";
const OTHER_SQL: &str = "SELECT row_to_json(r)::text FROM results r WHERE race_id = '2026-2-niigata-6-8R' ORDER BY result_id";
const RACES_SQL: &str = "SELECT row_to_json(r)::text FROM races r ORDER BY race_id";

/// 1 番の行だけがある対象レースと、別レース（全頭あり）を用意する。1 番は馬名を修正済み・scratched にしておく。
async fn setup(repo: &PostgresRepository) {
    repo.save_race_card(&card(RID, &[1, 2, 3])).await.unwrap();
    repo.save_race(&race(RID, vec![pdf_result(1)]))
        .await
        .unwrap();
    repo.save_race(&race(OTHER, vec![pdf_result(1), pdf_result(2)]))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE results SET horse_name = 'ウマ1改', status = 'scratched', horse_id = '2020100001', margin = 'クビ' \
         WHERE race_id = $1 AND horse_num = 1",
    )
    .bind(RID)
    .execute(&repo.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE races SET track_condition = '良', weather = '晴' WHERE race_id = $1")
        .bind(RID)
        .execute(&repo.pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn fills_only_missing_horse_nums_and_keeps_everything_else(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    setup(&repo).await;
    let existing_before = snapshot(&repo, EXISTING_SQL).await;
    let other_before = snapshot(&repo, OTHER_SQL).await;
    let races_before = snapshot(&repo, RACES_SQL).await;

    // 1 番（既存）も混ぜて渡す: DO NOTHING で既存行に触れないこと。4 番は出馬表に無い
    // 3 番は除外（着順なし）。取消・除外も status ごと入ることを確かめる
    let mut scratched = page_row(3, 3);
    scratched.status = ResultStatus::Scratched;
    scratched.finishing_position = None;
    let rows = vec![page_row(1, 1), page_row(2, 2), scratched, page_row(4, 4)];
    let inserted = repo
        .insert_missing_results(&card(RID, &[1, 2, 3]), &rows)
        .await
        .unwrap();
    assert_eq!(inserted, vec![2, 3]);

    assert_eq!(
        snapshot(&repo, EXISTING_SQL).await,
        existing_before,
        "既存行は全列そのまま"
    );
    assert_eq!(
        snapshot(&repo, OTHER_SQL).await,
        other_before,
        "他のレースは不変"
    );
    assert_eq!(
        snapshot(&repo, RACES_SQL).await,
        races_before,
        "races 行は不変"
    );

    // 馬番|枠|馬名|status|着順|source|horse_id|margin|騎手|単勝（gate_num と馬名は出馬表から・horse_id と margin は NULL）
    let new: Vec<(String,)> = sqlx::query_as(
        "SELECT concat_ws('|', horse_num, gate_num, horse_name, status, \
                coalesce(finishing_position::text, 'NULL'), source, \
                coalesce(horse_id, 'NULL'), coalesce(margin, 'NULL'), jockey, odds) \
         FROM results WHERE race_id = $1 AND horse_num IN (2, 3, 4) ORDER BY horse_num",
    )
    .bind(RID)
    .fetch_all(&repo.pool)
    .await
    .unwrap();
    let new: Vec<String> = new.into_iter().map(|r| r.0).collect();
    assert_eq!(
        new,
        vec![
            "2|1|ウマ2|finished|2|pdf|NULL|NULL|騎手A|12.3",
            "3|2|ウマ3|scratched|NULL|pdf|NULL|NULL|騎手A|12.3",
        ],
        "出馬表に無い 4 番は入れない"
    );

    // 2 回目は 0 行（冪等）
    let again = repo
        .insert_missing_results(&card(RID, &[1, 2, 3]), &rows)
        .await
        .unwrap();
    assert!(again.is_empty());
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn missing_races_row_is_an_error_and_writes_nothing(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // 出馬表だけあって races 行が無いレース
    repo.save_race_card(&card(RID, &[1, 2])).await.unwrap();
    let err = repo
        .insert_missing_results(&card(RID, &[1, 2]), &[page_row(1, 1)])
        .await
        .unwrap_err();
    assert!(err.to_string().contains("races"), "{err}");
    let n: (i64, i64) =
        sqlx::query_as("SELECT (SELECT count(*) FROM results), (SELECT count(*) FROM races)")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
    assert_eq!(n, (0, 0), "results にも races にも書かない");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn reads_existing_rows_for_fill(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    setup(&repo).await;
    let rows = repo
        .find_result_rows_for_fill(&RaceId::try_from(RID).unwrap())
        .await
        .unwrap();
    assert_eq!(
        rows,
        vec![ExistingResultRow {
            horse_num: 1,
            horse_name: "ウマ1改".into(),
            status: ResultStatus::Scratched,
            finishing_position: Some(1),
        }]
    );
    // sqlx::test が作った一時 DB の名前が返る（接続設定の DB 名と一致）
    let opts = repo.pool.connect_options();
    let expected = opts.get_database().map(str::to_string);
    assert_eq!(Some(repo.current_database().await.unwrap()), expected);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn race_exists_is_true_only_for_saved_races(pool: sqlx::PgPool) {
    // `SELECT 1` は INT4。i64 で受けていたため、行があると型の不一致で失敗していた（#742 で判明）
    let repo = PostgresRepository::new(pool);
    setup(&repo).await;
    assert!(
        repo.race_exists(&RaceId::try_from(RID).unwrap())
            .await
            .unwrap()
    );
    assert!(
        !repo
            .race_exists(&RaceId::try_from("2026-2-niigata-6-9R").unwrap())
            .await
            .unwrap()
    );
}
