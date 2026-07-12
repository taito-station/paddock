use std::collections::HashMap;

use chrono::{NaiveDate, NaiveTime};
use sqlx::PgPool;

use crate::error::Result;

/// 指定日の全レースの発走時刻を `race_id → post_time` で返す（`race_cards` 由来、#391）。
///
/// ライブ一覧の発走時刻・状態判定（未発走/終了）を watch 判定記録の有無に依存させず
/// DB 正本で行うための一括取得。post_time 未保存（NULL）や `HH:MM` として解釈できない
/// 行はマップに含めない（欠落＝不明として扱う）。
pub async fn find_post_times_by_date(
    pool: &PgPool,
    date: NaiveDate,
) -> Result<HashMap<String, NaiveTime>> {
    let date_str = date.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT race_id, post_time
        FROM race_cards
        WHERE date = $1
          AND post_time IS NOT NULL
        "#,
    )
    .bind(&date_str)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|(race_id, post_time)| {
            NaiveTime::parse_from_str(&post_time, "%H:%M")
                .ok()
                .map(|t| (race_id, t))
        })
        .collect())
}
