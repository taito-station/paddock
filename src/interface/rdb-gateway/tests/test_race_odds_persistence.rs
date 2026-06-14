//! `race_odds` の保存(save_race_odds)→読み出し(find_race_odds)を実 SQLite で往復検証する。
//! 単勝・複勝・組合せ券種(#38)の復元と、backtest 用の `as_of`（`date(fetched_at) <= d`）境界を担保する。

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use paddock_domain::{HorseNum, OrderedPair, OrderedTriple, Pair, RaceId, Triple};
use paddock_use_case::repository::{OddsRow, RaceOddsRecord, Repository};
use rdb_gateway::{SqliteRepository, pool};

async fn fresh_repo() -> (SqliteRepository, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let p = pool::connect(&url).await.expect("connect");
    pool::migrate(&p).await.expect("migrate");
    (SqliteRepository::new(p), dir)
}

fn race_id() -> RaceId {
    RaceId::try_from("2026-3-nakayama-8-1R").unwrap()
}

fn horse(n: u32) -> HorseNum {
    HorseNum::try_from(n).unwrap()
}

fn fetched_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 19, 10, 0, 0).unwrap()
}

/// 単勝 2 頭 + 複勝 2 頭を 1 レコードで保存する。
async fn save_sample(repo: &SqliteRepository) {
    let record = RaceOddsRecord {
        race_id: race_id(),
        fetched_at: fetched_at(),
        rows: vec![
            OddsRow {
                bet_type: "win".to_string(),
                combination_key: "1".to_string(),
                odds: 3.5,
                odds_high: None,
                popularity: Some(2),
            },
            OddsRow {
                bet_type: "win".to_string(),
                combination_key: "2".to_string(),
                odds: 7.1,
                odds_high: None,
                popularity: Some(5),
            },
            OddsRow {
                bet_type: "place".to_string(),
                combination_key: "1".to_string(),
                odds: 1.5,
                odds_high: Some(2.0),
                popularity: Some(2),
            },
            OddsRow {
                bet_type: "place".to_string(),
                combination_key: "2".to_string(),
                odds: 2.2,
                odds_high: Some(3.4),
                popularity: Some(5),
            },
        ],
    };
    repo.save_race_odds(&record).await.unwrap();
}

#[tokio::test]
async fn round_trips_win_and_place() {
    let (repo, _dir) = fresh_repo().await;
    save_sample(&repo).await;

    let odds = repo
        .find_race_odds(&race_id(), None)
        .await
        .unwrap()
        .expect("保存済みオッズが読めること");

    // 単勝: そのままの値で復元。
    assert_eq!(odds.win.len(), 2);
    assert!((odds.win.get(&horse(1)).unwrap().value() - 3.5).abs() < 1e-9);
    assert!((odds.win.get(&horse(2)).unwrap().value() - 7.1).abs() < 1e-9);

    // 複勝: odds=下限, odds_high=上限 として幅で復元。
    assert_eq!(odds.place.len(), 2);
    let p1 = odds.place.get(&horse(1)).unwrap();
    assert!((p1.low.value() - 1.5).abs() < 1e-9);
    assert!((p1.high.value() - 2.0).abs() < 1e-9);
}

#[tokio::test]
async fn round_trips_all_combination_bet_types() {
    let (repo, _dir) = fresh_repo().await;
    let pair = Pair::try_from((horse(1), horse(2))).unwrap();
    let opair = OrderedPair::try_from((horse(3), horse(1))).unwrap();
    let triple = Triple::try_from((horse(1), horse(2), horse(3))).unwrap();
    let otriple = OrderedTriple::try_from((horse(5), horse(2), horse(7))).unwrap();

    repo.save_race_odds(&RaceOddsRecord {
        race_id: race_id(),
        fetched_at: fetched_at(),
        rows: vec![
            OddsRow::quinella(pair, 12.4),
            OddsRow::wide(pair, 3.1, 4.8),
            OddsRow::exacta(opair, 25.0),
            OddsRow::trio(triple, 88.0),
            OddsRow::trifecta(otriple, 410.0),
        ],
    })
    .await
    .unwrap();

    let odds = repo
        .find_race_odds(&race_id(), None)
        .await
        .unwrap()
        .expect("保存済みオッズが読めること");

    // 馬連: 単一値、キーは昇順 Pair で復元。
    assert!((odds.quinella.get(&pair).unwrap().value() - 12.4).abs() < 1e-9);
    // ワイド: 複勝同様の幅 odds で復元。
    let w = odds.wide.get(&pair).unwrap();
    assert!((w.low.value() - 3.1).abs() < 1e-9 && (w.high.value() - 4.8).abs() < 1e-9);
    // 馬単: 順序が保持される（3>1 として復元）。
    assert!((odds.exacta.get(&opair).unwrap().value() - 25.0).abs() < 1e-9);
    // 三連複: 昇順 Triple。
    assert!((odds.trio.get(&triple).unwrap().value() - 88.0).abs() < 1e-9);
    // 三連単: 順序保持の OrderedTriple。
    assert!((odds.trifecta.get(&otriple).unwrap().value() - 410.0).abs() < 1e-9);
}

#[tokio::test]
async fn returns_none_when_absent() {
    let (repo, _dir) = fresh_repo().await;
    let other = RaceId::try_from("2026-3-nakayama-8-9R").unwrap();
    assert!(repo.find_race_odds(&other, None).await.unwrap().is_none());
}

#[tokio::test]
async fn upsert_keeps_existing_popularity_when_new_is_null() {
    let (repo, _dir) = fresh_repo().await;
    // 1) fetch-card 相当: 人気付きで保存。
    repo.save_race_odds(&RaceOddsRecord {
        race_id: race_id(),
        fetched_at: fetched_at(),
        rows: vec![OddsRow {
            bet_type: "win".to_string(),
            combination_key: "1".to_string(),
            odds: 3.5,
            odds_high: None,
            popularity: Some(2),
        }],
    })
    .await
    .unwrap();
    // 2) predict 相当: 同じキーを人気 None で再保存。odds は更新、人気は維持されるべき。
    repo.save_race_odds(&RaceOddsRecord {
        race_id: race_id(),
        fetched_at: fetched_at(),
        rows: vec![OddsRow {
            bet_type: "win".to_string(),
            combination_key: "1".to_string(),
            odds: 4.0,
            odds_high: None,
            popularity: None,
        }],
    })
    .await
    .unwrap();

    let (odds, popularity): (f64, Option<i64>) = sqlx::query_as(
        "SELECT odds, popularity FROM race_odds \
         WHERE race_id = $1 AND bet_type = 'win' AND combination_key = '1'",
    )
    .bind(race_id().value())
    .fetch_one(&repo.pool)
    .await
    .unwrap();
    assert!((odds - 4.0).abs() < 1e-9, "odds は最新で上書きされる");
    assert_eq!(popularity, Some(2), "人気は NULL で上書きされず維持される");
}

#[tokio::test]
async fn place_row_with_null_odds_high_is_data_error() {
    let (repo, _dir) = fresh_repo().await;
    // 保存側の不整合（複勝なのに odds_high が NULL）を直接 INSERT で再現し、
    // find_race_odds が幅 odds を組めず Error を返す防御経路を担保する。
    sqlx::query(
        "INSERT INTO race_odds (race_id, bet_type, combination_key, odds, odds_high, popularity, fetched_at) \
         VALUES ($1, 'place', '1', 1.5, NULL, NULL, $2)",
    )
    .bind(race_id().value())
    .bind(fetched_at().to_rfc3339())
    .execute(&repo.pool)
    .await
    .unwrap();

    assert!(
        repo.find_race_odds(&race_id(), None).await.is_err(),
        "複勝の odds_high が NULL なら Error を返す"
    );
}

#[tokio::test]
async fn unknown_bet_type_row_is_skipped_not_errored() {
    let (repo, _dir) = fresh_repo().await;
    save_sample(&repo).await; // win/place を投入
    // 将来券種を書く新版を模した未知 bet_type 行を直接 INSERT する。ラベルは BetType が将来
    // 拡張されても衝突しないダミーにする（実在馬券名を使うと拡張時にテストが意図せず壊れる）。
    // combination_key はラベルが未知の時点で評価されず skip されるので、内容は問わない。
    sqlx::query(
        "INSERT INTO race_odds (race_id, bet_type, combination_key, odds, odds_high, popularity, fetched_at) \
         VALUES ($1, '__unknown__', '1-2-3-4-5', 100.0, NULL, NULL, $2)",
    )
    .bind(race_id().value())
    .bind(fetched_at().to_rfc3339())
    .execute(&repo.pool)
    .await
    .unwrap();

    // 未知行はエラーにせず読み飛ばし、既知の win/place は通常どおり復元される。
    let odds = repo
        .find_race_odds(&race_id(), None)
        .await
        .unwrap()
        .expect("既知券種があるので Some");
    assert_eq!(odds.win.len(), 2);
    assert_eq!(odds.place.len(), 2);
}

#[tokio::test]
async fn invalid_odds_row_is_skipped_not_errored() {
    let (repo, _dir) = fresh_repo().await;
    save_sample(&repo).await; // win/place を投入
    // 旧版スクレイパの残骸を模した値域違反行（三連単 odds=0.0）を直接 INSERT する(#114)。
    // combination_key は妥当だが odds が OddsValue の下限(>=1.0)を割る。
    sqlx::query(
        "INSERT INTO race_odds (race_id, bet_type, combination_key, odds, odds_high, popularity, fetched_at) \
         VALUES ($1, 'trifecta', '3>1>2', 0.0, NULL, NULL, $2)",
    )
    .bind(race_id().value())
    .bind(fetched_at().to_rfc3339())
    .execute(&repo.pool)
    .await
    .unwrap();

    // 値域違反行はエラーにせず読み飛ばし（セッションを止めない）、既知の win/place は通常復元される。
    let odds = repo
        .find_race_odds(&race_id(), None)
        .await
        .unwrap()
        .expect("有効な win/place があるので Some");
    assert_eq!(odds.win.len(), 2);
    assert_eq!(odds.place.len(), 2);
    assert!(odds.trifecta.is_empty(), "0.0 の三連単行は読み飛ばされる");
}

#[tokio::test]
async fn band_invalid_odds_row_is_skipped_not_errored() {
    let (repo, _dir) = fresh_repo().await;
    save_sample(&repo).await; // win/place(有効) を投入
    // 幅 odds（複勝）の下限が値域違反（odds=0.0）だが odds_high は有効、というケースを直接 INSERT する。
    // parse_band の値域違反 skip 経路（Ok(None)）を担保する: 構造不正(odds_high NULL/low>high)の Err
    // とは別経路で、当該行のみ読み飛ばしセッションを止めないこと(#114)。
    sqlx::query(
        "INSERT INTO race_odds (race_id, bet_type, combination_key, odds, odds_high, popularity, fetched_at) \
         VALUES ($1, 'place', '5', 0.0, 2.0, NULL, $2)",
    )
    .bind(race_id().value())
    .bind(fetched_at().to_rfc3339())
    .execute(&repo.pool)
    .await
    .unwrap();

    let odds = repo
        .find_race_odds(&race_id(), None)
        .await
        .unwrap()
        .expect("有効な win/place があるので Some");
    // 値域違反の複勝行(馬番5)は読み飛ばされ、save_sample の有効な複勝2頭(馬番1,2)のみ残る。
    assert_eq!(odds.place.len(), 2);
    assert!(!odds.place.contains_key(&horse(5)), "0.0 下限の複勝行は skip される");
}

#[tokio::test]
async fn save_skips_invalid_odds_row() {
    let (repo, _dir) = fresh_repo().await;
    // 有効な単勝行 + 値域違反の三連単行(odds=0.0)を 1 レコードで保存する。
    // netkeiba の生 f64 経路を模し、保存側ガードが 0.0 を弾くことを担保する(#114)。
    let otriple = OrderedTriple::try_from((horse(3), horse(1), horse(2))).unwrap();
    repo.save_race_odds(&RaceOddsRecord {
        race_id: race_id(),
        fetched_at: fetched_at(),
        rows: vec![
            OddsRow {
                bet_type: "win".to_string(),
                combination_key: "1".to_string(),
                odds: 3.5,
                odds_high: None,
                popularity: None,
            },
            OddsRow::trifecta(otriple, 0.0),
        ],
    })
    .await
    .unwrap();

    // 0.0 行は INSERT されず、有効な win 行のみ保存される。
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM race_odds WHERE race_id = $1")
        .bind(race_id().value())
        .fetch_one(&repo.pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "値域違反行を除いた有効 1 行のみ保存される");
    let bet_type: String =
        sqlx::query_scalar("SELECT bet_type FROM race_odds WHERE race_id = $1")
            .bind(race_id().value())
            .fetch_one(&repo.pool)
            .await
            .unwrap();
    assert_eq!(bet_type, "win", "残るのは有効な win 行");
}

#[tokio::test]
async fn wide_row_with_null_odds_high_is_data_error() {
    let (repo, _dir) = fresh_repo().await;
    // ワイドは複勝同様の幅 odds。odds_high NULL は保存側不整合なので、place 同様に
    // 同一経路(parse_band)で Error になることを担保する。
    sqlx::query(
        "INSERT INTO race_odds (race_id, bet_type, combination_key, odds, odds_high, popularity, fetched_at) \
         VALUES ($1, 'wide', '1-2', 3.1, NULL, NULL, $2)",
    )
    .bind(race_id().value())
    .bind(fetched_at().to_rfc3339())
    .execute(&repo.pool)
    .await
    .unwrap();

    assert!(
        repo.find_race_odds(&race_id(), None).await.is_err(),
        "ワイドの odds_high が NULL なら Error を返す"
    );
}

#[tokio::test]
async fn as_of_filters_combination_rows() {
    let (repo, _dir) = fresh_repo().await;
    // 組合せ券種行にも as_of(date(fetched_at)<=d) のリーク防止境界が効くことを担保する。
    let pair = Pair::try_from((horse(1), horse(2))).unwrap();
    repo.save_race_odds(&RaceOddsRecord {
        race_id: race_id(),
        fetched_at: fetched_at(), // 2026-04-19
        rows: vec![OddsRow::quinella(pair, 12.4)],
    })
    .await
    .unwrap();

    let same_day = NaiveDate::from_ymd_opt(2026, 4, 19).unwrap();
    assert!(
        repo.find_race_odds(&race_id(), Some(same_day))
            .await
            .unwrap()
            .is_some_and(|o| o.quinella.contains_key(&pair)),
        "同日 as_of では当時オッズとして馬連が参照できる"
    );

    let day_before = NaiveDate::from_ymd_opt(2026, 4, 18).unwrap();
    assert!(
        repo.find_race_odds(&race_id(), Some(day_before))
            .await
            .unwrap()
            .is_none(),
        "as_of より後に取得された組合せ券種はリーク防止で除外される"
    );
}

#[tokio::test]
async fn as_of_filters_on_fetched_at_date() {
    let (repo, _dir) = fresh_repo().await;
    save_sample(&repo).await; // fetched_at = 2026-04-19

    // 当日以降の as_of は対象（date(fetched_at) <= as_of）。
    let same_day = NaiveDate::from_ymd_opt(2026, 4, 19).unwrap();
    assert!(
        repo.find_race_odds(&race_id(), Some(same_day))
            .await
            .unwrap()
            .is_some(),
        "fetched_at と同日は当時オッズとして参照できる"
    );

    // 前日の as_of では未来のスナップショット扱いで除外され None。
    let day_before = NaiveDate::from_ymd_opt(2026, 4, 18).unwrap();
    assert!(
        repo.find_race_odds(&race_id(), Some(day_before))
            .await
            .unwrap()
            .is_none(),
        "as_of より後に取得されたオッズはリーク防止で除外される"
    );
}
