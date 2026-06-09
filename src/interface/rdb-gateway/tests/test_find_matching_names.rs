//! #50: 馬名/騎手名の部分一致（中間一致）検索を実 SQLite で検証する。
//! - 中間一致で部分入力がヒット / 複数候補が返る / 正規化（半角カナ入力で全角格納名）/ limit
//! - 該当なしで空 / 騎手も同様

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, JockeyName, Race, RaceId,
    ResultStatus, Surface, Venue,
};
use paddock_use_case::repository::Repository;
use rdb_gateway::{SqliteRepository, pool};

async fn fresh_repo() -> (SqliteRepository, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let p = pool::connect(&url).await.expect("connect");
    pool::migrate(&p).await.expect("migrate");
    (SqliteRepository::new(p), dir)
}

/// 1 頭分の成績レース（horse_name は HorseName 正規化を通る）。
fn race(race_id: &str, horse: &str, jockey: &str) -> Race {
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
        venue: Venue::Tokyo,
        round: 3,
        day: 2,
        race_num: 1,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        weather: None,
        results: vec![HorseResult {
            finishing_position: Some(FinishingPosition::try_from(1u32).unwrap()),
            status: ResultStatus::Finished,
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(1u32).unwrap(),
            horse_name: HorseName::try_from(horse).unwrap(),
            horse_id: None,
            jockey: Some(JockeyName::try_from(jockey).unwrap()),
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

async fn seed(repo: &SqliteRepository) {
    // 「ダイワ」を含む 2 頭 + 無関係 1 頭。
    repo.save_race(&race("2026-3-tokyo-2-1R", "ダイワスカーレット", "ルメール"))
        .await
        .unwrap();
    repo.save_race(&race("2026-3-tokyo-2-2R", "ダイワメジャー", "横山和生"))
        .await
        .unwrap();
    repo.save_race(&race("2026-3-tokyo-2-3R", "イクイノックス", "ルメール"))
        .await
        .unwrap();
}

#[tokio::test]
async fn horse_partial_match_returns_multiple_sorted() {
    let (repo, _dir) = fresh_repo().await;
    seed(&repo).await;

    let hits = repo.find_matching_horse_names("ダイワ", 20).await.unwrap();
    assert_eq!(
        hits,
        vec!["ダイワスカーレット", "ダイワメジャー"],
        "中間一致・名前昇順"
    );

    // 中央の語でも中間一致でヒットする。
    let mid = repo
        .find_matching_horse_names("ノックス", 20)
        .await
        .unwrap();
    assert_eq!(mid, vec!["イクイノックス"]);
}

#[tokio::test]
async fn horse_query_is_normalized_halfwidth_kana() {
    let (repo, _dir) = fresh_repo().await;
    seed(&repo).await;

    // 半角カナ入力を HorseName で正規化 → 全角格納名にヒットする。
    let q = HorseName::try_from("ｲｸｲ").unwrap();
    let hits = repo.find_matching_horse_names(q.value(), 20).await.unwrap();
    assert_eq!(hits, vec!["イクイノックス"]);
}

#[tokio::test]
async fn horse_no_match_is_empty_and_limit_applies() {
    let (repo, _dir) = fresh_repo().await;
    seed(&repo).await;

    assert!(
        repo.find_matching_horse_names("該当なし馬", 20)
            .await
            .unwrap()
            .is_empty()
    );
    // limit=1 で 1 件に打ち切り。
    let one = repo.find_matching_horse_names("ダイワ", 1).await.unwrap();
    assert_eq!(one.len(), 1);
}

#[tokio::test]
async fn jockey_partial_match_and_distinct() {
    let (repo, _dir) = fresh_repo().await;
    seed(&repo).await;

    // ルメールは 2 レースに騎乗 → DISTINCT で 1 件。
    let hits = repo
        .find_matching_jockey_names("ルメール", 20)
        .await
        .unwrap();
    assert_eq!(hits, vec!["ルメール"]);

    let none = repo.find_matching_jockey_names("武豊", 20).await.unwrap();
    assert!(none.is_empty());
}
