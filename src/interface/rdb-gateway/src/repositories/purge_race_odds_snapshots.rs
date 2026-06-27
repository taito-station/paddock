use chrono::NaiveDate;
use sqlx::PgPool;

use crate::error::Result;

/// `fetched_at` の日付が `before` より前（厳密 `<`）の `race_odds_snapshots` 行を削除し、
/// 削除行数を返す（retention/パージ, #234）。
///
/// `fetched_at` は UTC rfc3339 TEXT（`+00:00` 固定で辞書順=時刻順）なので、先頭 10 文字
/// （`YYYY-MM-DD`）と cutoff 日付の TEXT 比較が UTC 日付比較になる（既存 `find_race_odds` の
/// `substr(fetched_at,1,10)` 規約と同一）。最新キャッシュ `race_odds` は対象にしない。
pub async fn purge_race_odds_snapshots(pool: &PgPool, before: NaiveDate) -> Result<u64> {
    let cutoff = before.format("%Y-%m-%d").to_string();
    let result = sqlx::query(
        r#"
        DELETE FROM race_odds_snapshots
        WHERE substr(fetched_at, 1, 10) < $1
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
        WHERE substr(fetched_at, 1, 10) < $1
        "#,
    )
    .bind(&cutoff)
    .fetch_one(pool)
    .await?;
    Ok(count as u64)
}
