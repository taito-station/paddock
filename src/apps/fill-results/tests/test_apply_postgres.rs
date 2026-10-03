//! `apply::run` を実 Postgres の `PostgresRepository` に通す（#742）。
//! FakeStore の単体テストではアダプタ（`impl FillStore for PostgresRepository`）の型の不一致などを検出できないため。

use fetch_final_odds::targets::Target;
use fill_results::apply;
use fill_results::fetch::html_path;
use paddock_domain::{
    FinishingPosition, GateNum, HorseEntry, HorseName, HorseNum, HorseResult, Race, RaceCard,
    RaceId, ResultStatus, Surface, Venue,
};
use paddock_use_case::repository::{RaceCardRepository, RaceRepository};
use rdb_gateway::PostgresRepository;

const RID: &str = "2026-2-niigata-6-7R";
const NO_RACES_ROW: &str = "2026-2-niigata-6-8R";

fn card(race_id: &str, nums: &[u32]) -> RaceCard {
    RaceCard {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: "2026-08-09".parse().unwrap(),
        post_time: None,
        venue: Venue::Niigata,
        round: 2,
        day: 6,
        race_num: 7,
        surface: Surface::Turf,
        distance: 1600,
        race_class: None,
        race_name: None,
        entries: nums
            .iter()
            .map(|&n| HorseEntry {
                gate_num: GateNum::try_from(n).unwrap(),
                horse_num: HorseNum::try_from(n).unwrap(),
                horse_name: HorseName::try_from(format!("ウマ{n}")).unwrap(),
                jockey: None,
                trainer: None,
                weight_carried: None,
            })
            .collect(),
    }
}

fn race_with_first(race_id: &str) -> Race {
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: "2026-08-09".parse().unwrap(),
        venue: Venue::Niigata,
        round: 2,
        day: 6,
        race_num: 7,
        surface: Surface::Turf,
        distance: 1600,
        track_condition: None,
        weather: None,
        results: vec![HorseResult {
            finishing_position: Some(FinishingPosition::try_from(1u32).unwrap()),
            status: ResultStatus::Finished,
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(1u32).unwrap(),
            horse_name: HorseName::try_from("ウマ1").unwrap(),
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

/// `parse_race_result` が読める最小の結果ページ（1〜3 着）。
fn page() -> String {
    let trs: String = (1..=3)
        .map(|n| {
            format!(r#"<tr><td class="Result_Num">{n}</td><td class="Num Txt_C">{n}</td></tr>"#)
        })
        .collect();
    format!(r#"<html><body><table id="All_Result_Table">{trs}</table></body></html>"#)
}

fn target(race_id: &str, netkeiba_id: &str) -> Target {
    Target {
        race_id: race_id.into(),
        netkeiba_id: netkeiba_id.into(),
    }
}

async fn result_nums(repo: &PostgresRepository, race_id: &str) -> Vec<i64> {
    let rows: Vec<(i64,)> =
        sqlx::query_as("SELECT horse_num FROM results WHERE race_id = $1 ORDER BY horse_num")
            .bind(race_id)
            .fetch_all(&repo.pool)
            .await
            .unwrap();
    rows.into_iter().map(|r| r.0).collect()
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn apply_runs_against_postgres(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    repo.save_race_card(&card(RID, &[1, 2, 3])).await.unwrap();
    repo.save_race(&race_with_first(RID)).await.unwrap();
    // 出馬表はあるが races 行の無いレース
    repo.save_race_card(&card(NO_RACES_ROW, &[1, 2, 3]))
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("raw")).unwrap();
    for rid in [RID, NO_RACES_ROW] {
        std::fs::write(html_path(dir.path(), rid), page()).unwrap();
    }
    let targets = [
        target(RID, "202604020607"),
        target(NO_RACES_ROW, "202604020608"),
    ];

    let dry = apply::run(&repo, &targets, dir.path(), true).await;
    assert_eq!(dry.races.len(), 1, "{:?}", dry.failures);
    assert_eq!(dry.races[0].planned, vec![2, 3]);
    assert_eq!(dry.races[0].inserted, None);
    assert_eq!(dry.failures.len(), 1);
    assert!(dry.failures[0].1.contains("races 行"), "{:?}", dry.failures);
    assert_eq!(result_nums(&repo, RID).await, vec![1], "dry-run は書かない");

    let real = apply::run(&repo, &targets, dir.path(), false).await;
    assert_eq!(real.races[0].inserted, Some(vec![2, 3]));
    assert_eq!(real.failures.len(), 1, "races 行の無いレースは書かない");
    assert_eq!(result_nums(&repo, RID).await, vec![1, 2, 3]);
    assert!(result_nums(&repo, NO_RACES_ROW).await.is_empty());
}
