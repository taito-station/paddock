//! 再取り込みの非破壊性 (#61) を Postgres で検証する:
//! - `save_race` の再取り込みが backfill 済み `horse_id` を温存すること（全消し DELETE 廃止）
//! - 今回の出走集合に無い馬番（取消・除外）だけが掃除されること
//! - `save_race_card` も同様に非破壊で、消えた馬番のみ掃除されること

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseEntry, HorseName, HorseNum, HorseResult, Race, RaceCard,
    RaceId, ResultStatus, Surface, TrainerName, Venue,
};
use paddock_use_case::repository::Repository;
use rdb_gateway::PostgresRepository;

fn d() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, 19).unwrap()
}

fn result(horse_num: u32, horse: &str) -> HorseResult {
    HorseResult {
        finishing_position: Some(FinishingPosition::try_from(horse_num).unwrap()),
        status: ResultStatus::Finished,
        gate_num: GateNum::try_from(1u32).unwrap(),
        horse_num: HorseNum::try_from(horse_num).unwrap(),
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
    }
}

fn race(race_id: &str, results: Vec<HorseResult>) -> Race {
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: d(),
        venue: Venue::Nakayama,
        round: 3,
        day: 8,
        race_num: 1,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        weather: None,
        results,
    }
}

async fn horse_id_of(repo: &PostgresRepository, race_id: &str, horse_num: i64) -> Option<String> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT horse_id FROM results WHERE race_id = $1 AND horse_num = $2")
            .bind(race_id)
            .bind(horse_num)
            .fetch_optional(&repo.pool)
            .await
            .unwrap();
    row.and_then(|r| r.0)
}

async fn result_horse_nums(repo: &PostgresRepository, race_id: &str) -> Vec<i64> {
    let rows: Vec<(i64,)> =
        sqlx::query_as("SELECT horse_num FROM results WHERE race_id = $1 ORDER BY horse_num")
            .bind(race_id)
            .fetch_all(&repo.pool)
            .await
            .unwrap();
    rows.into_iter().map(|r| r.0).collect()
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn reingest_preserves_backfilled_horse_id(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let rid = "2026-3-nakayama-8-1R";
    repo.save_race(&race(rid, vec![result(1, "ウマA"), result(2, "ウマB")]))
        .await
        .unwrap();

    // #60 の backfill を模して horse_id を後入れする（pdf は horse_id を持たない）。
    sqlx::query("UPDATE results SET horse_id = $1 WHERE race_id = $2 AND horse_num = 1")
        .bind("2020100001")
        .bind(rid)
        .execute(&repo.pool)
        .await
        .unwrap();

    // 同じ PDF を再取り込み（Race の horse_id は None のまま）。
    repo.save_race(&race(rid, vec![result(1, "ウマA"), result(2, "ウマB")]))
        .await
        .unwrap();

    assert_eq!(
        horse_id_of(&repo, rid, 1).await.as_deref(),
        Some("2020100001"),
        "再取り込みで backfill 済み horse_id が消えてはいけない"
    );
    assert_eq!(result_horse_nums(&repo, rid).await, vec![1, 2]);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn reingest_removes_only_absent_horse_nums(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let rid = "2026-3-nakayama-8-1R";
    repo.save_race(&race(rid, vec![result(1, "ウマA"), result(2, "ウマB")]))
        .await
        .unwrap();
    sqlx::query("UPDATE results SET horse_id = $1 WHERE race_id = $2 AND horse_num = 1")
        .bind("2020100001")
        .bind(rid)
        .execute(&repo.pool)
        .await
        .unwrap();

    // 2 番が出走取消 → 今回は 1 番のみ。
    repo.save_race(&race(rid, vec![result(1, "ウマA")]))
        .await
        .unwrap();

    assert_eq!(
        result_horse_nums(&repo, rid).await,
        vec![1],
        "今回いない 2 番だけが掃除される"
    );
    assert_eq!(
        horse_id_of(&repo, rid, 1).await.as_deref(),
        Some("2020100001"),
        "残った 1 番の horse_id は保持される"
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn reingest_with_empty_results_keeps_existing_rows(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let rid = "2026-3-nakayama-8-1R";
    repo.save_race(&race(rid, vec![result(1, "ウマA"), result(2, "ウマB")]))
        .await
        .unwrap();

    // 劣化パース等で results が空のレースを再取り込みしても、既存行は消さない（全消し防御）。
    repo.save_race(&race(rid, vec![])).await.unwrap();

    assert_eq!(
        result_horse_nums(&repo, rid).await,
        vec![1, 2],
        "空 results の再取り込みで既存行を消さない"
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn save_race_card_reingest_removes_only_absent_entries(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let rid = "2026-3-nakayama-8-1R";
    let entry = |n: u32| HorseEntry {
        gate_num: GateNum::try_from(1u32).unwrap(),
        horse_num: HorseNum::try_from(n).unwrap(),
        horse_name: HorseName::try_from(format!("ウマ{n}")).unwrap(),
        jockey: None,
        trainer: None,
        weight_carried: None,
    };
    let card = |entries: Vec<HorseEntry>| RaceCard {
        race_id: RaceId::try_from(rid).unwrap(),
        date: d(),
        venue: Venue::Nakayama,
        round: 3,
        day: 8,
        race_num: 1,
        surface: Surface::Turf,
        distance: 1800,
        entries,
    };

    repo.save_race_card(&card(vec![entry(1), entry(2), entry(3)]))
        .await
        .unwrap();
    repo.save_race_card(&card(vec![entry(1), entry(3)]))
        .await
        .unwrap();

    let loaded = repo
        .find_race_card(&RaceId::try_from(rid).unwrap())
        .await
        .unwrap()
        .unwrap();
    let mut nums: Vec<u32> = loaded.entries.iter().map(|e| e.horse_num.value()).collect();
    nums.sort_unstable();
    assert_eq!(nums, vec![1, 3], "出走取消の 2 番だけが掃除される");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn save_race_card_coalesce_keeps_trainer_from_netkeiba(pool: sqlx::PgPool) {
    // trainer は netkeiba 経路のみが埋める。PDF 経路（trainer=None）が後から同じ race_id を
    // 書いても、COALESCE により netkeiba が入れた trainer が消えないことを検証する（#74）。
    let repo = PostgresRepository::new(pool);
    let rid = "2026-3-nakayama-8-2R";
    let make_card = |trainer: Option<&str>| RaceCard {
        race_id: RaceId::try_from(rid).unwrap(),
        date: d(),
        venue: Venue::Nakayama,
        round: 3,
        day: 8,
        race_num: 2,
        surface: Surface::Turf,
        distance: 1800,
        entries: vec![HorseEntry {
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(1u32).unwrap(),
            horse_name: HorseName::try_from("ウマA").unwrap(),
            jockey: None,
            trainer: trainer.map(|t| TrainerName::try_from(t).unwrap()),
            weight_carried: None,
        }],
    };

    // netkeiba 経路（trainer あり）→ PDF 経路（trainer None）の順で上書きする。
    repo.save_race_card(&make_card(Some("田中博")))
        .await
        .unwrap();
    repo.save_race_card(&make_card(None)).await.unwrap();

    let loaded = repo
        .find_race_card(&RaceId::try_from(rid).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.entries[0].trainer.as_ref().map(|t| t.value()),
        Some("田中博"),
        "PDF 経路の None が netkeiba の trainer を消してはいけない（COALESCE）"
    );
}
