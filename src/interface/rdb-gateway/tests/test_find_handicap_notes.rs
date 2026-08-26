//! `horse_handicap_notes`（#628・盤の手動ハンデ精査材料）を Postgres で検証する。
//!
//! 核心は「**交差条件**（場 × 芝ダ × 距離）を数えていること」——既存の `horse_stats` が返す
//! 周辺分布（`by_surface` / `by_distance_band` / `by_venue`）では表せない集合なので、
//! 距離違い・芝ダ違い・別場が混ざらないことを個別に張る。

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

fn name(s: &str) -> HorseName {
    HorseName::try_from(s).unwrap()
}

/// netkeiba 近走 1 走。場・芝ダ・距離・着順を個別に振れるようにして交差条件を試す。
#[allow(clippy::too_many_arguments)]
fn nk_run(
    nk_id: &str,
    horse: &str,
    date: NaiveDate,
    race_num: u32,
    venue: Venue,
    surface: Surface,
    distance: u32,
    finish: Option<u32>,
) -> HorsePastRun {
    HorsePastRun {
        netkeiba_race_id: nk_id.to_string(),
        date,
        venue,
        round: 1,
        day: 1,
        race_num,
        surface,
        distance,
        track_condition: None,
        finishing_position: finish.map(|f| FinishingPosition::try_from(f).unwrap()),
        status: if finish.is_some() {
            ResultStatus::Finished
        } else {
            ResultStatus::Scratched
        },
        gate_num: GateNum::try_from(1u32).unwrap(),
        horse_num: HorseNum::try_from(1u32).unwrap(),
        horse_name: name(horse),
        jockey: None,
        time_seconds: None,
        margin: None,
        odds: None,
        horse_weight: None,
        weight_change: None,
        weight_carried: None,
        popularity: None,
        race_name: Some("テストS".to_string()),
        corner_positions: None,
        field_size: None,
    }
}

/// pdf 確定成績 1 レース（1 頭ぶん）。
#[allow(clippy::too_many_arguments)]
fn pdf_race(
    race_id: &str,
    date: NaiveDate,
    race_num: u32,
    venue: Venue,
    surface: Surface,
    distance: u32,
    horse: &str,
    finish: u32,
) -> Race {
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date,
        venue,
        round: 1,
        day: 1,
        race_num,
        surface,
        distance,
        track_condition: None,
        weather: None,
        results: vec![HorseResult {
            finishing_position: Some(FinishingPosition::try_from(finish).unwrap()),
            status: ResultStatus::Finished,
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(1u32).unwrap(),
            horse_name: name(horse),
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

fn horse_id(s: &str) -> HorseId {
    HorseId::try_from(s.to_string()).unwrap()
}

/// 交差条件（場 × 芝ダ × 距離）だけが `exact_runs` に入る。周辺分布では区別できない
/// 「同場だが別距離」「同距離だが別場」「同場同距離だが芝ダ違い」を個別に排除する。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn exact_runs_require_all_three_conditions(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    repo.upsert_horse_history(
        &horse_id("2020100001"),
        &[
            // 完全一致（新潟・芝・1000）＝千直。2 走。
            nk_run(
                "202604020101",
                "ウマA",
                ymd(2026, 7, 1),
                1,
                Venue::Niigata,
                Surface::Turf,
                1000,
                Some(3),
            ),
            nk_run(
                "202604020102",
                "ウマA",
                ymd(2026, 5, 1),
                2,
                Venue::Niigata,
                Surface::Turf,
                1000,
                Some(8),
            ),
            // 同場・同芝ダだが距離違い（1200）。
            nk_run(
                "202604020103",
                "ウマA",
                ymd(2026, 6, 1),
                3,
                Venue::Niigata,
                Surface::Turf,
                1200,
                Some(1),
            ),
            // 同場・同距離だが芝ダ違い（ダート）。
            nk_run(
                "202604020104",
                "ウマA",
                ymd(2026, 4, 1),
                4,
                Venue::Niigata,
                Surface::Dirt,
                1000,
                Some(1),
            ),
            // 同芝ダ・同距離だが別場（中京）。
            nk_run(
                "202607020105",
                "ウマA",
                ymd(2026, 3, 1),
                5,
                Venue::Chukyo,
                Surface::Turf,
                1000,
                Some(1),
            ),
        ],
    )
    .await
    .unwrap();

    let notes = repo
        .horse_handicap_notes(
            &[name("ウマA")],
            Venue::Niigata,
            Surface::Turf,
            1000,
            Some(ymd(2026, 8, 16)),
        )
        .await
        .unwrap();
    let n = &notes[&name("ウマA")];

    assert_eq!(n.exact_runs.len(), 2, "千直 2 走だけが完全一致");
    // date 降順。着順・レース名も運ぶ。
    assert_eq!(n.exact_runs[0].date, ymd(2026, 7, 1));
    assert_eq!(n.exact_runs[0].finishing_position, 3);
    assert_eq!(n.exact_runs[0].race_name.as_deref(), Some("テストS"));
    assert_eq!(n.exact_runs[1].finishing_position, 8);

    // 新潟は洋芝でないのでグループ＝自身 1 場＝完全一致と同じ集合。
    assert_eq!(n.group_runs, n.exact_runs);

    // 周辺分布の値は交差条件とは別に数える。
    assert_eq!(n.total_starts, 5);
    assert_eq!(n.same_surface_starts, 4, "芝は 4 走（ダート 1 走を除く）");
    // 距離の経験は場・芝ダを問わず数える（「今回距離を走ったことがあるか」だけを見る指標）。
    // 1000m は 新潟芝 2 走＋新潟ダ 1 走＋中京芝 1 走＝4 走。1200m は ±100 の範囲外。
    assert_eq!(n.same_distance_starts, 4);
    assert_eq!(n.last_run_date, Some(ymd(2026, 7, 1)));
}

/// 洋芝（札幌⇄函館）は `group_runs` で 1 グループに束ねるが、`exact_runs` は当場のみ。
/// 「完全一致は該当なしだが洋芝では走っている」を別行で出せることが要件（#628）。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn yoshiba_group_widens_only_group_runs(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    repo.upsert_horse_history(
        &horse_id("2020100002"),
        &[
            // 函館・芝・2000（今回は札幌なので完全一致ではない）。
            nk_run(
                "202602010101",
                "ウマB",
                ymd(2026, 6, 14),
                1,
                Venue::Hakodate,
                Surface::Turf,
                2000,
                Some(5),
            ),
            // 札幌・芝・2000（完全一致）。
            nk_run(
                "202601010102",
                "ウマB",
                ymd(2026, 7, 5),
                2,
                Venue::Sapporo,
                Surface::Turf,
                2000,
                Some(1),
            ),
            // 洋芝だが距離違い＝グループにも入らない。
            nk_run(
                "202602010103",
                "ウマB",
                ymd(2026, 5, 5),
                3,
                Venue::Hakodate,
                Surface::Turf,
                1800,
                Some(2),
            ),
        ],
    )
    .await
    .unwrap();

    let notes = repo
        .horse_handicap_notes(
            &[name("ウマB")],
            Venue::Sapporo,
            Surface::Turf,
            2000,
            Some(ymd(2026, 8, 16)),
        )
        .await
        .unwrap();
    let n = &notes[&name("ウマB")];

    assert_eq!(n.exact_runs.len(), 1, "当場（札幌）のみが完全一致");
    assert_eq!(n.exact_runs[0].finishing_position, 1);
    assert_eq!(n.group_runs.len(), 2, "洋芝グループは札幌＋函館の同条件");
    // group_runs は exact_runs の上位集合で date 降順。
    assert_eq!(n.group_runs[0].date, ymd(2026, 7, 5));
    assert_eq!(n.group_runs[1].date, ymd(2026, 6, 14));
    assert!(
        n.group_runs.contains(&n.exact_runs[0]),
        "group は exact を包含する"
    );
}

/// 着順の無い行（取消・除外）は「走っていない」ので母集団に入れない。
/// `as_of` より後の走りも入れない（確定後の盤で自レースが自分の過去走に混ざらない）。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn excludes_scratched_and_future_runs(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    repo.upsert_horse_history(
        &horse_id("2020100003"),
        &[
            nk_run(
                "202604020201",
                "ウマC",
                ymd(2026, 7, 1),
                1,
                Venue::Niigata,
                Surface::Turf,
                1000,
                Some(4),
            ),
            // 取消（着順なし）。
            nk_run(
                "202604020202",
                "ウマC",
                ymd(2026, 6, 1),
                2,
                Venue::Niigata,
                Surface::Turf,
                1000,
                None,
            ),
            // as_of 当日の走り（＝今回のレースそのもの）。
            nk_run(
                "202604020203",
                "ウマC",
                ymd(2026, 8, 16),
                3,
                Venue::Niigata,
                Surface::Turf,
                1000,
                Some(1),
            ),
        ],
    )
    .await
    .unwrap();

    let notes = repo
        .horse_handicap_notes(
            &[name("ウマC")],
            Venue::Niigata,
            Surface::Turf,
            1000,
            Some(ymd(2026, 8, 16)),
        )
        .await
        .unwrap();
    let n = &notes[&name("ウマC")];

    assert_eq!(n.exact_runs.len(), 1, "取消と当日走は数えない");
    assert_eq!(n.exact_runs[0].date, ymd(2026, 7, 1));
    assert_eq!(n.total_starts, 1);
    assert_eq!(n.last_run_date, Some(ymd(2026, 7, 1)));
}

/// pdf 確定成績と netkeiba 近走の同一実レースは 1 走に dedup し、**着順は netkeiba を採る**。
///
/// 二重計上すると「千直 2 走」が「4 走」に見えて判断材料が壊れる。加えて、pdf は既知のパーサ
/// 制約（EdiF フォントで着順カラムが欠落）で着順が 1 つズレることがあり、実測で両ソースの
/// 11.1%（3,503/31,585 走）が食い違う——うち 76% が `pdf = netkeiba + 1`。人が読む着順を
/// 出す経路なのでここでは netkeiba を優先する（スコア経路 `find_recent_runs` は pdf 優先のまま）。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn dedups_same_race_and_prefers_netkeiba_position(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // 同一実レース（新潟 1 回 1 日 6R・2026-07-01）が pdf と netkeiba の両方にある。
    repo.save_race(&pdf_race(
        "2026-1-niigata-1-6R",
        ymd(2026, 7, 1),
        6,
        Venue::Niigata,
        Surface::Turf,
        1000,
        "ウマD",
        2,
    ))
    .await
    .unwrap();
    repo.upsert_horse_history(
        &horse_id("2020100004"),
        &[
            nk_run(
                "202604010106",
                "ウマD",
                ymd(2026, 7, 1),
                6,
                Venue::Niigata,
                Surface::Turf,
                1000,
                Some(9),
            ),
            // netkeiba にしかない別レース。
            nk_run(
                "202604010107",
                "ウマD",
                ymd(2026, 6, 1),
                7,
                Venue::Niigata,
                Surface::Turf,
                1000,
                Some(6),
            ),
        ],
    )
    .await
    .unwrap();

    let notes = repo
        .horse_handicap_notes(
            &[name("ウマD")],
            Venue::Niigata,
            Surface::Turf,
            1000,
            Some(ymd(2026, 8, 16)),
        )
        .await
        .unwrap();
    let n = &notes[&name("ウマD")];

    assert_eq!(n.exact_runs.len(), 2, "同一実レースは 1 走に dedup");
    assert_eq!(n.total_starts, 2);
    assert_eq!(
        n.exact_runs[0].finishing_position, 9,
        "着順は netkeiba を採る（pdf の 2 着ではない＝着順ズレを持ち込まない）"
    );
    assert_eq!(
        n.exact_runs[0].race_name.as_deref(),
        Some("テストS"),
        "netkeiba 側の行が残るのでレース名も残る（pdf 経路は race_name を持たない）"
    );
}

/// 過去走が 1 走も無い馬も map に含める。呼び出し側が「該当なし（走っていない）」と
/// 「材料なし（引けていない）」を取り違えないための境界。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn includes_horses_without_any_past_run(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let notes = repo
        .horse_handicap_notes(
            &[name("ウマE"), name("ウマF"), name("ウマE")],
            Venue::Niigata,
            Surface::Turf,
            1000,
            None,
        )
        .await
        .unwrap();

    assert_eq!(notes.len(), 2, "重複馬名は 1 エントリに畳む");
    for horse in ["ウマE", "ウマF"] {
        let n = &notes[&name(horse)];
        assert_eq!(n.total_starts, 0);
        assert!(n.exact_runs.is_empty());
        assert!(n.group_runs.is_empty());
        assert_eq!(n.last_run_date, None);
        assert_eq!(n.same_surface_starts, 0);
        assert_eq!(n.same_distance_starts, 0);
    }
}
