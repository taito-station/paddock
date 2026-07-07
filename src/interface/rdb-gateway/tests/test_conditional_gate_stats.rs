//! #343: 条件依存枠バイアス集計（`conditional_gate_stats`）を Postgres（`#[sqlx::test]` の一時 DB）で
//! 検証する。馬場(良/非良)フィルタ・頭数帯(field_size=race 単位 COUNT 由来)・枠群グルーピングが
//! 期待どおり効くこと、非該当セルが空になることを seed 済みデータで突き合わせる。

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, RaceId, ResultStatus, Surface,
    TrackCondition, Venue,
};
use paddock_use_case::repository::{RaceRepository, StatsRepository};
use rdb_gateway::PostgresRepository;

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn result(finish: u32, gate: u32, horse_num: u32) -> HorseResult {
    HorseResult {
        finishing_position: Some(FinishingPosition::try_from(finish).unwrap()),
        status: ResultStatus::Finished,
        gate_num: GateNum::try_from(gate).unwrap(),
        horse_num: HorseNum::try_from(horse_num).unwrap(),
        horse_name: HorseName::try_from(format!("ウマ{horse_num}")).unwrap(),
        horse_id: None,
        jockey: None,
        trainer: None,
        time_seconds: None,
        margin: None,
        odds: None,
        horse_weight: Some(480),
        weight_change: Some(0),
        weight_carried: None,
        popularity: Some(horse_num),
    }
}

fn race(race_id: &str, tc: TrackCondition, results: Vec<HorseResult>) -> paddock_domain::Race {
    paddock_domain::Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: ymd(2026, 1, 10),
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 1,
        surface: Surface::Turf,
        distance: 1600,
        track_condition: Some(tc),
        weather: None,
        results,
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn conditional_gate_stats_filters_track_field_and_gate(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);

    // 良・6頭(少帯 ≤9)。内枠(1-3)=3頭、うち複勝圏(top3)は 1着・2着の2頭。中(4)=1頭・外(7)=1頭・(8)=1頭。
    let firm = race(
        "202606050101",
        TrackCondition::Firm,
        vec![
            result(1, 1, 1), // 内・複勝
            result(2, 2, 2), // 内・複勝
            result(5, 3, 3), // 内・圏外
            result(3, 4, 4), // 中・複勝
            result(4, 7, 5), // 外
            result(6, 8, 6), // 外
        ],
    );
    // 非良(重)・同コース・少帯。良フィルタで除外されることの対照。
    let yielding = race(
        "202606050102",
        TrackCondition::Yielding,
        vec![result(1, 1, 1), result(2, 2, 2), result(3, 4, 3)],
    );
    repo.save_race(&firm).await.unwrap();
    repo.save_race(&yielding).await.unwrap();

    let row = repo
        .conditional_gate_stats(Venue::Nakayama, 1600, Surface::Turf, None)
        .await
        .unwrap();

    // 良・少(-9)・内枠: 3 走・複勝 2（gate1,2）。
    let inner = row
        .cell("良", "少(-9)", "Inner (1-3)")
        .expect("良・少・内枠セルがある");
    assert_eq!(inner.stat.starts, 3, "内枠は 3 走");
    assert_eq!(inner.stat.shows, 2, "内枠の複勝は 2");

    // 同条件の全枠平均複勝率＝(2+1+0)/6。lift 基準線が集計できる。
    let base = row.condition_show_rate("良", "少(-9)").unwrap();
    assert!(
        (base - 3.0 / 6.0).abs() < 1e-9,
        "全枠平均複勝率 base={base}"
    );
    // 内枠 lift = 2/3 − 3/6 = +0.1667。
    assert!((inner.stat.show_rate() - base - (2.0 / 3.0 - 0.5)).abs() < 1e-9);

    // 非良の内枠は良フィルタで除外され良セルに混ざらない（良・内枠は firm のみ由来の 3 走）。
    // 頭数帯フィルタ: この 2 レースは少帯なので 多(14-18) は空。
    assert_eq!(
        row.cell("良", "多(14-18)", "Inner (1-3)")
            .unwrap()
            .stat
            .starts,
        0,
        "多帯セルは空（両レースとも少帯）"
    );
    // 馬場フィルタ: 非良・少・内枠は yielding 由来の 2 走（gate1,2）。
    assert_eq!(
        row.cell("非良", "少(-9)", "Inner (1-3)")
            .unwrap()
            .stat
            .starts,
        2,
        "非良・少・内枠は yielding の 2 走"
    );
}
