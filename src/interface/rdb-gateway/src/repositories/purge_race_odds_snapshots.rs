use chrono::NaiveDate;
use sqlx::PgPool;

use crate::error::Result;

/// `fetched_at` の日付が `before` より前（厳密 `<`）の `race_odds_snapshots` 行を削除し、
/// 削除行数を返す（retention/パージ, #234）。
///
/// `fetched_at` は UTC rfc3339 TEXT（`+00:00` 固定で辞書順=時刻順）なので、cutoff 日付
/// （`YYYY-MM-DD`）との TEXT 比較 `fetched_at < $1` が UTC 日付の厳密 `<` になる。
/// 関数（`substr`）を外した直接比較にすることで sargable にし、将来 index を活用可能にする。
///
/// 同値性: 差が出るのは `substr(fetched_at,1,10) == cutoff`（日付部が cutoff と一致）の行のみ。
/// その行は `fetched_at == cutoff || "T…"` なので、共通接頭辞 `cutoff` に `"T…"` が続く分だけ
/// `fetched_at > cutoff`（長い方が後）となり `fetched_at < cutoff` は false → 旧述語同様 **残る**。
/// 他の行は日付部の大小がそのまま `fetched_at` の大小に一致するため、削除集合は完全に同一。
/// 最新キャッシュ `race_odds` は対象にしない。
pub async fn purge_race_odds_snapshots(pool: &PgPool, before: NaiveDate) -> Result<u64> {
    let cutoff = before.format("%Y-%m-%d").to_string();
    let result = sqlx::query(
        r#"
        DELETE FROM race_odds_snapshots
        WHERE fetched_at < $1
        "#,
    )
    .bind(&cutoff)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// `purge_race_odds_snapshots` の削除対象行数を、削除せずに数える（dry-run 用, #234）。
pub async fn count_race_odds_snapshots_before(pool: &PgPool, before: NaiveDate) -> Result<u64> {
    let cutoff = before.format("%Y-%m-%d").to_string();
    let (count,): (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM race_odds_snapshots
        WHERE fetched_at < $1
        "#,
    )
    .bind(&cutoff)
    .fetch_one(pool)
    .await?;
    Ok(count as u64)
}
