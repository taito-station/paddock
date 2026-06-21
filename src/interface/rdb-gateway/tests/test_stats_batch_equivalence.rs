//! #196: バッチ統計取得（`*_batch`）が per-item 版と完全同値であることを Postgres で保証する
//! リグレッションテスト。backtest 高速化はこの同値性に全面的に依存するため、SQL を書き換えても
//! per-item と 1 件もズレないことを seed 済みデータで突き合わせる。比較は構造一致＝Debug 文字列一致
//! で行う（row 型は PartialEq 非 derive のため）。

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseId, HorseName, HorseNum, HorseResult, JockeyName, Race,
    RaceId, ResultStatus, Surface, TrackCondition, TrainerName, Venue,
};
use paddock_use_case::HorsePastRun;
use paddock_use_case::repository::{HorseHistoryRepository, RaceRepository, StatsRepository};
use rdb_gateway::PostgresRepository;

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn result(
    horse: &str,
    finish: u32,
    gate: u32,
    horse_num: u32,
    popularity: u32,
    jockey: Option<&str>,
    trainer: Option<&str>,
) -> HorseResult {
    HorseResult {
        finishing_position: Some(FinishingPosition::try_from(finish).unwrap()),
        status: ResultStatus::Finished,
        gate_num: GateNum::try_from(gate).unwrap(),
        horse_num: HorseNum::try_from(horse_num).unwrap(),
        horse_name: HorseName::try_from(horse).unwrap(),
        horse_id: None,
        jockey: jockey.map(|j| JockeyName::try_from(j).unwrap()),
        trainer: trainer.map(|t| TrainerName::try_from(t).unwrap()),
        time_seconds: None,
        margin: None,
        odds: None,
        horse_weight: Some(480),
        weight_change: Some(0),
        weight_carried: None,
        popularity: Some(popularity),
    }
}

#[allow(clippy::too_many_arguments)]
fn race(
    race_id: &str,
    date: NaiveDate,
    surface: Surface,
    distance: u32,
    track_condition: TrackCondition,
    results: Vec<HorseResult>,
) -> Race {
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date,
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 1,
        surface,
        distance,
        track_condition: Some(track_condition),
        weather: None,
        results,
    }
}

/// netkeiba 近走 1 走を作る（`upsert_horse_history` 経由で `horse_past_runs` に入る）。dedup は
/// `(date, venue, race_num)` を horse_name 相関で見るため、その 3 つが pdf レースと一致すれば
/// 同一実レースとして突き合わされる（race_id は netkeiba 12 桁→canonical 変換で別物になる）。
#[allow(clippy::too_many_arguments)]
fn past_run(
    netkeiba_race_id: &str,
    horse: &str,
    date: NaiveDate,
    race_num: u32,
    surface: Surface,
    distance: u32,
    finish: u32,
) -> HorsePastRun {
    HorsePastRun {
        netkeiba_race_id: netkeiba_race_id.to_string(),
        date,
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num,
        surface,
        distance,
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

/// 馬・騎手・調教師が複数レース・複数 surface/距離帯/枠/人気帯/馬場状態にまたがるコーパスを作る。
/// 同一レースに複数馬を入れて recent_runs の per-horse dedup（horse_name 相関）も効かせる。
/// track_condition は良/重/良 と非 NULL の異なる値を設定し、`by_track_condition` を実値で検証する。
/// さらに netkeiba 近走（horse_past_runs）も seed し、recent_runs のクロスソース dedup
/// （UNION ALL 第2ブランチ + src_rank）を batch==per-item で突き合わせる。
async fn seed(repo: &PostgresRepository) {
    // r1: 芝1200(短距離) 内枠 / 良。A=1着/1人気, B=3着/4人気（同一レース2頭）。
    repo.save_race(&race(
        "r1",
        ymd(2025, 5, 10),
        Surface::Turf,
        1200,
        TrackCondition::Firm, // 良
        vec![
            result("ウマA", 1, 2, 1, 1, Some("騎手P"), Some("調教師Q")),
            result("ウマB", 3, 3, 2, 4, Some("騎手P"), Some("調教師R")),
        ],
    ))
    .await
    .unwrap();
    // r2: ダート1800(中距離) 中枠 / 重。A=5着/6人気。
    repo.save_race(&race(
        "r2",
        ymd(2025, 8, 20),
        Surface::Dirt,
        1800,
        TrackCondition::Yielding, // 重
        vec![result("ウマA", 5, 5, 1, 6, Some("騎手S"), Some("調教師Q"))],
    ))
    .await
    .unwrap();
    // r3: 芝2400(長距離) 外枠 / 良。B=1着/2人気, A=2着/11人気（同一レース2頭）。
    repo.save_race(&race(
        "r3",
        ymd(2026, 1, 15),
        Surface::Turf,
        2400,
        TrackCondition::Firm, // 良
        vec![
            result("ウマB", 1, 7, 1, 2, Some("騎手P"), Some("調教師R")),
            result("ウマA", 2, 8, 2, 11, Some("騎手S"), Some("調教師Q")),
        ],
    ))
    .await
    .unwrap();

    // netkeiba 近走（ウマA）。horse_past_runs は集計（horse_stats/horse_recency）には効かず、
    // recent_runs の UNION 第2ブランチにのみ現れる。
    // (a) r2 と同一実レース(2025-8-20, 中山, race_num=1)に重複する netkeiba 走。dedup で
    //     pdf(src_rank 0) が優先され、netkeiba(src_rank 1) は recent_runs に出ないはず。
    //     canonical race_id は別物（202506020801 → 2025-2-nakayama-8-1R ≠ "r2"）。
    // (b) pdf に無い netkeiba 単独走(2024-12-01, 中山, race_num=1)。recent_runs に現れるはず。
    let horse_id = HorseId::try_from("2019104567".to_string()).unwrap();
    repo.upsert_horse_history(
        &horse_id,
        &[
            past_run(
                "202506020801",
                "ウマA",
                ymd(2025, 8, 20),
                1,
                Surface::Dirt,
                1800,
                4,
            ),
            past_run(
                "202406010101",
                "ウマA",
                ymd(2024, 12, 1),
                1,
                Surface::Turf,
                1600,
                7,
            ),
        ],
    )
    .await
    .unwrap();
}

/// 各エントリで batch と per-item を突き合わせる。`as_of` は None と Some(将来日) の両方で検証する。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn stats_batch_matches_per_item(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    seed(&repo).await;

    let horses: Vec<HorseName> = ["ウマA", "ウマB", "ウマZ"] // ウマZ は不在（ゼロ合成）
        .iter()
        .map(|n| HorseName::try_from(*n).unwrap())
        .collect();
    let jockeys: Vec<JockeyName> = ["騎手P", "騎手S", "騎手Z"]
        .iter()
        .map(|n| JockeyName::try_from(*n).unwrap())
        .collect();
    let trainers: Vec<TrainerName> = ["調教師Q", "調教師R", "調教師Z"]
        .iter()
        .map(|n| TrainerName::try_from(*n).unwrap())
        .collect();

    for as_of in [None, Some(ymd(2026, 6, 1))] {
        // horse_stats
        let batch = repo.horse_stats_batch(&horses, as_of).await.unwrap();
        for h in &horses {
            let single = repo.horse_stats(h, as_of).await.unwrap();
            assert_eq!(
                format!("{:?}", batch.get(h).expect("batch covers all names")),
                format!("{single:?}"),
                "horse_stats_batch != horse_stats for {h:?} as_of={as_of:?}"
            );
        }
        // horse_recency
        let rbatch = repo.horse_recency_batch(&horses, as_of).await.unwrap();
        for h in &horses {
            let single = repo.horse_recency(h, as_of).await.unwrap();
            assert_eq!(
                format!("{:?}", rbatch.get(h).expect("batch covers all names")),
                format!("{single:?}"),
                "horse_recency_batch != horse_recency for {h:?} as_of={as_of:?}"
            );
        }
        // jockey_stats
        let jbatch = repo.jockey_stats_batch(&jockeys, as_of).await.unwrap();
        for j in &jockeys {
            let single = repo.jockey_stats(j, as_of).await.unwrap();
            assert_eq!(
                format!("{:?}", jbatch.get(j).expect("batch covers all names")),
                format!("{single:?}"),
                "jockey_stats_batch != jockey_stats for {j:?} as_of={as_of:?}"
            );
        }
        // trainer_stats
        let tbatch = repo.trainer_stats_batch(&trainers, as_of).await.unwrap();
        for t in &trainers {
            let single = repo.trainer_stats(t, as_of).await.unwrap();
            assert_eq!(
                format!("{:?}", tbatch.get(t).expect("batch covers all names")),
                format!("{single:?}"),
                "trainer_stats_batch != trainer_stats for {t:?} as_of={as_of:?}"
            );
        }
    }
}

/// recent_runs_batch が find_recent_runs（馬ごと）と同一順・同一内容であること。同一レースに複数馬が
/// 居るコーパスで、別馬の同レースが互いを dedup し合わないこと（horse_name 相関）を含めて検証する。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn recent_runs_batch_matches_per_item(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    seed(&repo).await;

    let horses: Vec<HorseName> = ["ウマA", "ウマB", "ウマZ"]
        .iter()
        .map(|n| HorseName::try_from(*n).unwrap())
        .collect();
    let before = ymd(2026, 6, 1);

    for limit in [1u32, 5u32] {
        let batch = repo
            .recent_runs_batch(&horses, before, limit)
            .await
            .unwrap();
        for h in &horses {
            let single = repo.find_recent_runs(h, before, limit).await.unwrap();
            assert_eq!(
                format!("{:?}", batch.get(h).expect("batch covers all names")),
                format!("{single:?}"),
                "recent_runs_batch != find_recent_runs for {h:?} limit={limit}"
            );
        }
    }
}
