//! `pool::connect` が SQLite 接続に意図した PRAGMA を適用していることを実 SQLite で検証する。
//! 特に #120 で追加した `busy_timeout`（ロック競合時の即時失敗を 5s のリトライ待ちへ緩和）を担保する。

use rdb_gateway::pool;

#[tokio::test]
async fn connect_applies_busy_timeout_and_existing_pragmas() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let p = pool::connect(&url).await.expect("connect");

    // #120: busy_timeout = 5s = 5000ms。並走プロセスのロック即時失敗を緩和する。
    let busy: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(busy, 5000, "busy_timeout は 5000ms");

    // 既存のプール設定（FK 有効・WAL）が retain されていることも併せて確認する。
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(fk, 1, "foreign_keys は有効");
    let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(journal, "wal", "journal_mode は WAL");
}
