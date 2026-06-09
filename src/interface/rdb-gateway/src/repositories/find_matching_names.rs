use sqlx::SqlitePool;

use crate::error::Result;

/// LIKE のワイルドカード（`%` / `_`）とエスケープ文字（`\`）をリテラル化する。
/// `query` 中にこれらが混じっても任意一致せず、入力文字そのものとして検索する。
fn escape_like(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// `query` を中間一致（`LIKE '%query%'`）する馬名を、stats の出る `results` から重複排除して
/// 名前昇順で最大 `limit` 件返す（`analyze` の部分一致検索用）。`query` は正規化済み前提。
///
/// 前方ワイルドカードのため `horse_name` の index は使われずフルスキャンになるが、`results`
/// 規模では許容する（大規模化したら FTS 等を検討）。
pub async fn find_matching_horse_names(
    pool: &SqlitePool,
    query: &str,
    limit: u32,
) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT horse_name
        FROM results
        WHERE horse_name LIKE '%' || ? || '%' ESCAPE '\'
        ORDER BY horse_name
        LIMIT ?
        "#,
    )
    .bind(escape_like(query))
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
          AND jockey LIKE '%' || ? || '%' ESCAPE '\'
        ORDER BY jockey
        LIMIT ?
        "#,
    )
    .bind(escape_like(query))
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}
