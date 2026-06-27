//! `race_odds` の保存(save_race_odds)→読み出し(find_race_odds)を Postgres で往復検証する。
//! 単勝・複勝・組合せ券種(#38)の復元と、backtest 用の `as_of`（`substr(fetched_at,1,10) <= d`）境界を担保する。

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use paddock_domain::{HorseNum, OrderedPair, OrderedTriple, Pair, RaceId, Triple};
use paddock_use_case::repository::{OddsRepository, OddsRow, RaceOddsRecord};
use rdb_gateway::PostgresRepository;

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
async fn save_sample(repo: &PostgresRepository) {
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn round_trips_win_and_place(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn round_trips_all_combination_bet_types(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn returns_none_when_absent(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let other = RaceId::try_from("2026-3-nakayama-8-9R").unwrap();
    assert!(repo.find_race_odds(&other, None).await.unwrap().is_none());
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn upsert_keeps_existing_popularity_when_new_is_null(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn place_row_with_null_odds_high_is_data_error(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn unknown_bet_type_row_is_skipped_not_errored(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn invalid_odds_row_is_skipped_not_errored(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn band_invalid_odds_row_is_skipped_not_errored(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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
    assert!(
        !odds.place.contains_key(&horse(5)),
        "0.0 下限の複勝行は skip される"
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn save_skips_invalid_odds_row(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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
    let bet_type: String = sqlx::query_scalar("SELECT bet_type FROM race_odds WHERE race_id = $1")
        .bind(race_id().value())
        .fetch_one(&repo.pool)
        .await
        .unwrap();
    assert_eq!(bet_type, "win", "残るのは有効な win 行");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn save_skips_row_with_invalid_odds_high(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // 下限は有効だが上限が値域違反（odds_high=0.0）の複勝行。保存ガードの
    // `odds_high.is_some_and(is_invalid_odds)` 分岐が弾くことを担保する(#114)。
    repo.save_race_odds(&RaceOddsRecord {
        race_id: race_id(),
        fetched_at: fetched_at(),
        rows: vec![OddsRow::place(7, 1.5, 0.0, None)],
    })
    .await
    .unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM race_odds WHERE race_id = $1")
        .bind(race_id().value())
        .fetch_one(&repo.pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "上限のみ値域違反でも行ごと弾く");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn save_keeps_inverted_band_which_read_rejects(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // 下限・上限とも値域内だが low>high の複勝行。保存側ガードは値域のみ見るため**弾かず**、
    // 読み取り側 parse_band が構造不正として Err を返す——という意図的な非対称性を回帰固定する。
    // band 構造不正は「保存できるが読めない」検知すべき不正状態として stop させる設計(#114)。
    repo.save_race_odds(&RaceOddsRecord {
        race_id: race_id(),
        fetched_at: fetched_at(),
        rows: vec![OddsRow::place(7, 3.0, 2.0, None)],
    })
    .await
    .unwrap();

    // 値域内なので保存はされる。
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM race_odds WHERE race_id = $1")
        .bind(race_id().value())
        .fetch_one(&repo.pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "low>high でも値域内なら保存側は通す");

    // 読み取り側は構造不正として Err（skip せず stop で早期検知）。
    assert!(
        repo.find_race_odds(&race_id(), None).await.is_err(),
        "low>high の複勝行は読み取り時に Error"
    );
}

// 旧 `cleanup_migration_deletes_only_invalid_rows` は、一回限りのデータ修正 migration
// (20260614000001_cleanup_invalid_race_odds) の挙動を検証していたが、Postgres ベースライン
// への集約でその migration ファイルは無くなった（新規 DB には不正行が存在しない）。不正オッズ
// 行を弾く現行の不変条件は save ガード側のテスト（`save_skips_invalid_odds_row` 等）が担保する。

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn wide_row_with_null_odds_high_is_data_error(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn as_of_filters_combination_rows(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // 組合せ券種行にも as_of(substr(fetched_at,1,10)<=d) のリーク防止境界が効くことを担保する。
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn as_of_filters_on_fetched_at_date(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    save_sample(&repo).await; // fetched_at = 2026-04-19

    // 当日以降の as_of は対象（substr(fetched_at,1,10) <= as_of）。
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

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn snapshots_retain_history_across_fetches(pool: sqlx::PgPool) {
    // race_odds は最新値で上書きされるが、race_odds_snapshots は fetched_at ごとに履歴を
    // 残す（#232）。締切前 live が後続/事後フェッチで消えないことを担保する。
    let repo = PostgresRepository::new(pool);

    // 1) 締切前 live 相当: odds=3.5 を 10:00 に保存。
    repo.save_race_odds(&RaceOddsRecord {
        race_id: race_id(),
        fetched_at: Utc.with_ymd_and_hms(2026, 4, 19, 10, 0, 0).unwrap(),
        rows: vec![OddsRow {
            bet_type: "win".to_string(),
            combination_key: "1".to_string(),
            odds: 3.5,
            odds_high: None,
            popularity: None,
        }],
    })
    .await
    .unwrap();

    // 2) 事後フェッチ相当: 同一キーを別時刻 12:00 に odds=4.0 で再保存。
    repo.save_race_odds(&RaceOddsRecord {
        race_id: race_id(),
        fetched_at: Utc.with_ymd_and_hms(2026, 4, 19, 12, 0, 0).unwrap(),
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

    // race_odds は最新値の単一行（既存キャッシュ挙動は不変）。
    let cache: Vec<f64> = sqlx::query_scalar(
        "SELECT odds FROM race_odds \
         WHERE race_id = $1 AND bet_type = 'win' AND combination_key = '1'",
    )
    .bind(race_id().value())
    .fetch_all(&repo.pool)
    .await
    .unwrap();
    assert_eq!(cache, vec![4.0], "race_odds は最新値1行のみ");

    // snapshots は両時刻の履歴を保持し、live(3.5) が事後フェッチ(4.0)で消えない。
    let history: Vec<f64> = sqlx::query_scalar(
        "SELECT odds FROM race_odds_snapshots \
         WHERE race_id = $1 AND bet_type = 'win' AND combination_key = '1' \
         ORDER BY fetched_at",
    )
    .bind(race_id().value())
    .fetch_all(&repo.pool)
    .await
    .unwrap();
    assert_eq!(history, vec![3.5, 4.0], "別時刻の取得が時系列で両方残る");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn snapshots_idempotent_on_same_fetched_at(pool: sqlx::PgPool) {
    // 同一 fetched_at の再保存は履歴を増やさない（ON CONFLICT DO NOTHING の冪等性）。
    let repo = PostgresRepository::new(pool);
    let record = || RaceOddsRecord {
        race_id: race_id(),
        fetched_at: fetched_at(),
        rows: vec![OddsRow {
            bet_type: "win".to_string(),
            combination_key: "1".to_string(),
            odds: 3.5,
            odds_high: None,
            popularity: None,
        }],
    };
    repo.save_race_odds(&record()).await.unwrap();
    repo.save_race_odds(&record()).await.unwrap();

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM race_odds_snapshots \
         WHERE race_id = $1 AND bet_type = 'win' AND combination_key = '1'",
    )
    .bind(race_id().value())
    .fetch_one(&repo.pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "同一 fetched_at の再保存で履歴は重複しない");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn snapshots_skip_invalid_odds_rows(pool: sqlx::PgPool) {
    // 値域違反行は race_odds 同様 snapshots にも入らない（保存ガードの内側で両テーブルへ書くため）。
    let repo = PostgresRepository::new(pool);
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
            OddsRow::trifecta(otriple, 0.0), // 値域違反
        ],
    })
    .await
    .unwrap();

    // 残る 1 行が（弾かれた trifecta ではなく）有効な win 行であることまで確認する。
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT bet_type, combination_key FROM race_odds_snapshots WHERE race_id = $1",
    )
    .bind(race_id().value())
    .fetch_all(&repo.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![("win".to_string(), "1".to_string())],
        "snapshots に残るのは有効な win 行のみ（trifecta 0.0 は弾かれる）"
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn snapshots_persist_band_odds_high(pool: sqlx::PgPool) {
    // 幅 odds（複勝・ワイドの odds_high）も snapshots に二重書きされることを担保する。
    let repo = PostgresRepository::new(pool);
    repo.save_race_odds(&RaceOddsRecord {
        race_id: race_id(),
        fetched_at: fetched_at(),
        rows: vec![OddsRow::place(1, 1.5, 2.0, None)],
    })
    .await
    .unwrap();

    let (odds, odds_high): (f64, Option<f64>) = sqlx::query_as(
        "SELECT odds, odds_high FROM race_odds_snapshots \
         WHERE race_id = $1 AND bet_type = 'place' AND combination_key = '1'",
    )
    .bind(race_id().value())
    .fetch_one(&repo.pool)
    .await
    .unwrap();
    assert!((odds - 1.5).abs() < 1e-9, "下限が snapshots に残る");
    assert_eq!(
        odds_high.map(|h| (h - 2.0).abs() < 1e-9),
        Some(true),
        "上限(odds_high)も snapshots に残る"
    );
}

/// 指定 UTC 日時の単勝 1 頭レコードを保存する（retention テスト用）。
async fn save_win_at(repo: &PostgresRepository, at: DateTime<Utc>, odds: f64) {
    repo.save_race_odds(&RaceOddsRecord {
        race_id: race_id(),
        fetched_at: at,
        rows: vec![OddsRow {
            bet_type: "win".to_string(),
            combination_key: "1".to_string(),
            odds,
            odds_high: None,
            popularity: None,
        }],
    })
    .await
    .unwrap();
}

async fn snapshots_count(repo: &PostgresRepository) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM race_odds_snapshots WHERE race_id = $1")
        .bind(race_id().value())
        .fetch_one(&repo.pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn purge_deletes_old_snapshots_keeps_recent(pool: sqlx::PgPool) {
    // retention(#234): cutoff より前の fetched_at の snapshots だけ削除し、保持期間内は残す。
    // race_odds（最新キャッシュ）は消さない。
    let repo = PostgresRepository::new(pool);
    let old = Utc.with_ymd_and_hms(2025, 1, 1, 10, 0, 0).unwrap();
    let recent = Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap();
    save_win_at(&repo, old, 3.5).await;
    save_win_at(&repo, recent, 4.0).await; // 同一キー別時刻 → snapshots 2 行 / race_odds 最新1行
    assert_eq!(snapshots_count(&repo).await, 2, "前提: snapshots は 2 行");

    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    // dry-run(count) は削除せず対象行数（古い 1 行）を返す。
    assert_eq!(
        repo.count_race_odds_snapshots_before(cutoff).await.unwrap(),
        1
    );
    assert_eq!(snapshots_count(&repo).await, 2, "count は削除しない");

    // 実削除: 古い 1 行のみ削除。
    assert_eq!(repo.purge_race_odds_snapshots(cutoff).await.unwrap(), 1);
    assert_eq!(snapshots_count(&repo).await, 1, "保持期間内の 1 行が残る");
    assert_eq!(
        repo.count_race_odds_snapshots_before(cutoff).await.unwrap(),
        0,
        "古い行は消えた"
    );

    // 残った snapshot は recent(2026-06-01)。
    let remaining: String = sqlx::query_scalar(
        "SELECT substr(fetched_at,1,10) FROM race_odds_snapshots WHERE race_id=$1",
    )
    .bind(race_id().value())
    .fetch_one(&repo.pool)
    .await
    .unwrap();
    assert_eq!(remaining, "2026-06-01");

    // race_odds（最新キャッシュ）は purge の影響を受けず、最新値 4.0 のまま読める。
    let odds = repo
        .find_race_odds(&race_id(), None)
        .await
        .unwrap()
        .expect("race_odds は残る");
    assert!((odds.win.get(&horse(1)).unwrap().value() - 4.0).abs() < 1e-9);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn purge_is_strict_before_cutoff(pool: sqlx::PgPool) {
    // cutoff 当日（date(fetched_at) == cutoff）は残す（厳密 `<`）。翌日 cutoff で削除される。
    let repo = PostgresRepository::new(pool);
    save_win_at(
        &repo,
        Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap(),
        3.5,
    )
    .await;

    let same_day = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    assert_eq!(repo.purge_race_odds_snapshots(same_day).await.unwrap(), 0);
    assert_eq!(snapshots_count(&repo).await, 1, "cutoff 当日は残る");

    let next_day = NaiveDate::from_ymd_opt(2026, 6, 2).unwrap();
    assert_eq!(repo.purge_race_odds_snapshots(next_day).await.unwrap(), 1);
    assert_eq!(snapshots_count(&repo).await, 0, "翌日 cutoff で削除される");
}
