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

    // 単一 GROUP BY が 1 クエリで複数セルを同時に分割集計することをロック（中枠・外枠も別セルへ）。
    let middle = row
        .cell("良", "少(-9)", "Middle (4-6)")
        .expect("良・少・中枠セルがある");
    assert_eq!(middle.stat.starts, 1, "中枠は gate4 の 1 走");
    assert_eq!(middle.stat.shows, 1, "中枠 gate4 は 3 着で複勝");
    let outer = row
        .cell("良", "少(-9)", "Outer (7-8)")
        .expect("良・少・外枠セルがある");
    assert_eq!(outer.stat.starts, 2, "外枠は gate7,8 の 2 走");
    assert_eq!(outer.stat.shows, 0, "外枠は複勝圏外(4,6着)");

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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn conditional_gate_stats_respects_as_of_cutoff(pool: sqlx::PgPool) {
    // date フィルタ（`as_of` = `races.date < d`）はバックテストのリーク防止の高感度点。
    // #358 で per-query WHERE から `target_races` CTE へ移設したため、cutoff 以降のレースが
    // 集計から除外されることを回帰でロックする。
    let repo = PostgresRepository::new(pool);

    // 同一コース・良・少帯・内枠。cutoff より前(1/1)と後(2/1)に 1 レースずつ。
    let mut early = race(
        "202606050301",
        TrackCondition::Firm,
        vec![result(1, 1, 1), result(2, 2, 2), result(4, 3, 3)],
    );
    early.date = ymd(2026, 1, 1);
    let mut late = race(
        "202606050302",
        TrackCondition::Firm,
        vec![result(1, 1, 1), result(2, 2, 2), result(3, 3, 3)],
    );
    late.date = ymd(2026, 2, 1);
    repo.save_race(&early).await.unwrap();
    repo.save_race(&late).await.unwrap();

    // cutoff = 1/15。early(1/1) のみ算入され late(2/1) は除外される。
    let row = repo
        .conditional_gate_stats(Venue::Nakayama, 1600, Surface::Turf, Some(ymd(2026, 1, 15)))
        .await
        .unwrap();
    let inner = row
        .cell("良", "少(-9)", "Inner (1-3)")
        .expect("良・少・内枠セルがある");
    assert_eq!(
        inner.stat.starts, 3,
        "early の 3 走のみ（late は cutoff で除外）"
    );
    assert_eq!(
        inner.stat.shows, 2,
        "early の複勝 2（着1,2）。late 分は混ざらない"
    );

    // cutoff 無し（None）だと両レース算入＝6 走。移設した date フィルタが効いていることの対照。
    let all = repo
        .conditional_gate_stats(Venue::Nakayama, 1600, Surface::Turf, None)
        .await
        .unwrap();
    assert_eq!(
        all.cell("良", "少(-9)", "Inner (1-3)").unwrap().stat.starts,
        6,
        "cutoff 無しなら early+late の 6 走"
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn conditional_gate_stats_many_band_positive_via_field_size_count(pool: sqlx::PgPool) {
    // 多帯(14-18)の positive 検証: field_size は race 単位 COUNT で導出されるため、14 走の良レースは
    // 多帯セルに入り、同レースは少帯セルには入らない（頭数帯フィルタが COUNT で効くこと）。
    let repo = PostgresRepository::new(pool);
    // 良・14頭・全馬 枠2(内枠 1-3)。複勝(top3)は着1,2,3 の 3 頭。
    let results: Vec<HorseResult> = (1..=14).map(|n| result(n, 2, n)).collect();
    repo.save_race(&race("202606050201", TrackCondition::Firm, results))
        .await
        .unwrap();

    let row = repo
        .conditional_gate_stats(Venue::Nakayama, 1600, Surface::Turf, None)
        .await
        .unwrap();

    // 多帯・良・内枠: 14 走・複勝 3。COUNT=14 が多帯(14-18)に落ちることの positive 確認。
    assert_eq!(
        row.cell("良", "多(14-18)", "Inner (1-3)")
            .unwrap()
            .stat
            .starts,
        14,
        "14 頭は多帯(COUNT 由来)"
    );
    assert_eq!(
        row.cell("良", "多(14-18)", "Inner (1-3)")
            .unwrap()
            .stat
            .shows,
        3,
        "複勝は着1,2,3 の 3 頭"
    );
    // 同レースは少帯には入らない（COUNT=14 は少(-9) の範囲外）。
    assert_eq!(
        row.cell("良", "少(-9)", "Inner (1-3)").unwrap().stat.starts,
        0,
        "14 頭レースは少帯に入らない"
    );
}
