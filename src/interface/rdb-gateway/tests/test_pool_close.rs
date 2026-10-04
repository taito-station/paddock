//! pool を閉じてから終える（#717）ことを、サーバ側に残ったバックエンドの数で検証する。
//!
//! 共有 DB は Lima のポート転送越しで、クライアントが Terminate を送らずに切断すると、サーバ側に
//! idle のバックエンドが残り続ける。`PgPool::close` は Terminate を送るので、閉じれば残らない。
//! 各テストは別接続（observer）から `pg_stat_activity` を見て、一時 DB に自分以外の接続が
//! 0 本になるのを待つ（Terminate 後のバックエンド終了は非同期なのでポーリングする）。

use std::time::{Duration, Instant};

use rdb_gateway::pool;
use sqlx::{Connection, PgConnection, PgPool};

static EMPTY: sqlx::migrate::Migrator = sqlx::migrate::Migrator::DEFAULT;

/// 一時 DB に自分（observer）以外の接続が `0` 本になるまで最大 5 秒待ち、最後に見た本数を返す。
async fn other_backends_after_wait(observer: &mut PgConnection) -> i64 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() AND pid <> pg_backend_pid() \
             AND backend_type = 'client backend'",
        )
        .fetch_one(&mut *observer)
        .await
        .unwrap();
        if n == 0 || Instant::now() >= deadline {
            return n;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn observer_for(pool: &PgPool) -> PgConnection {
    PgConnection::connect_with(&pool.connect_options())
        .await
        .unwrap()
}

/// `DATABASE_URL` の DB 名だけを一時 DB に差し替えた URL（`connect_checked` は URL 文字列を取るため）。
/// クエリ文字列（`sslmode` や README の `options`）は残す。
async fn temp_db_url(pool: &PgPool) -> String {
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(pool)
        .await
        .unwrap();
    let base = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let (without_query, query) = base
        .split_once('?')
        .map_or((base.as_str(), None), |(u, q)| (u, Some(q)));
    let authority = without_query
        .find("://")
        .expect("DATABASE_URL に scheme が無い")
        + 3;
    let slash = without_query[authority..]
        .rfind('/')
        .map(|i| authority + i)
        .expect("DATABASE_URL に DB 名が無い");
    let mut url = format!("{}/{db}", &without_query[..slash]);
    if let Some(q) = query {
        url.push('?');
        url.push_str(q);
    }
    url
}

#[sqlx::test(migrator = "EMPTY")]
async fn close_after_closes_pool_when_ok(pool: PgPool) {
    let mut observer = observer_for(&pool).await;
    let out = pool::close_after(&pool, async {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&pool)
            .await
    })
    .await;
    assert_eq!(out.unwrap(), 1);
    assert!(pool.is_closed());
    assert_eq!(other_backends_after_wait(&mut observer).await, 0);
    observer.close().await.unwrap();
}

#[sqlx::test(migrator = "EMPTY")]
async fn close_after_closes_pool_when_err(pool: PgPool) {
    let mut observer = observer_for(&pool).await;
    let out: Result<(), &str> = pool::close_after(&pool, async {
        sqlx::query("SELECT 1").execute(&pool).await.unwrap();
        Err("本体の失敗")
    })
    .await;
    assert_eq!(out, Err("本体の失敗"));
    assert!(pool.is_closed());
    assert_eq!(other_backends_after_wait(&mut observer).await, 0);
    observer.close().await.unwrap();
}

/// `connect_checked` が Uninitialized で Err を返すとき、自分で作った pool を閉じてから返す。
///
/// 注: pool を閉じずに drop しても、DB へ直接つながる環境（CI）ではソケットの切断がサーバに届くので、
/// この検査は修正前でも通る。RED になるのは、切断が届かない Lima のポート転送越し（ローカルの共有 DB）だけ。
#[sqlx::test(migrator = "EMPTY")]
async fn connect_checked_closes_pool_when_uninitialized(pool: PgPool) {
    sqlx::query("DROP TABLE IF EXISTS _sqlx_migrations")
        .execute(&pool)
        .await
        .unwrap();
    let url = temp_db_url(&pool).await;
    let mut observer = observer_for(&pool).await;
    pool.close().await;
    assert_eq!(other_backends_after_wait(&mut observer).await, 0);

    let err = pool::connect_checked(&url, false).await.unwrap_err();
    assert!(err.to_string().contains("未初期化"), "{err}");
    assert_eq!(other_backends_after_wait(&mut observer).await, 0);
    observer.close().await.unwrap();
}

/// `connect_checked` が Pending で Err を返すとき（未適用の migration がある間の単発 bin）も閉じる。
///
/// 注: 上の Uninitialized のテストと同じく、RED になるのは Lima のポート転送越しだけ。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn connect_checked_closes_pool_when_pending(pool: PgPool) {
    sqlx::query(
        "DELETE FROM _sqlx_migrations WHERE version = (SELECT max(version) FROM _sqlx_migrations)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let url = temp_db_url(&pool).await;
    let mut observer = observer_for(&pool).await;
    pool.close().await;
    assert_eq!(other_backends_after_wait(&mut observer).await, 0);

    let err = pool::connect_checked(&url, false).await.unwrap_err();
    assert!(err.to_string().contains("未適用"), "{err}");
    assert_eq!(other_backends_after_wait(&mut observer).await, 0);
    observer.close().await.unwrap();
}
