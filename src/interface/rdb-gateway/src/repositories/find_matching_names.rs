use sqlx::SqlitePool;

use crate::error::Result;

/// `query` を中間一致（`LIKE '%query%'`）する馬名を、stats の出る `results` から重複排除して
/// 名前昇順で最大 `limit` 件返す（`analyze` の部分一致検索用）。`query` は正規化済み前提。
pub async fn find_matching_horse_names(
    pool: &SqlitePool,
    query: &str,
    limit: u32,
) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT horse_name
        FROM results
        WHERE horse_name LIKE '%' || ? || '%'
        ORDER BY horse_name
        LIMIT ?
        "#,
    )
    .bind(query)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// 騎手名版（[`find_matching_horse_names`] と同方針）。`results.jockey` が NULL の行は除外する。
pub async fn find_matching_jockey_names(
    pool: &SqlitePool,
    query: &str,
    limit: u32,
) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT jockey
        FROM results
        WHERE jockey IS NOT NULL
          AND jockey LIKE '%' || ? || '%'
        ORDER BY jockey
        LIMIT ?
        "#,
    )
    .bind(query)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}
