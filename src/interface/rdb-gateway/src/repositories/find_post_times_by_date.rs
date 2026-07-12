use std::collections::HashMap;

use chrono::{NaiveDate, NaiveTime};
use paddock_domain::RaceId;
use sqlx::PgPool;

use crate::error::Result;

/// 指定日の全レースの発走時刻を `race_id → post_time` で返す（`race_cards` 由来、#391）。
///
/// ライブ一覧の発走時刻・状態判定（未発走/終了）を watch 判定記録の有無に依存させず
/// DB 正本で行うための一括取得。post_time 未保存（NULL）は SQL で除外（warn なし・正常系）。
/// `HH:MM` や race_id として解釈できない行のみ warn を出してマップから除外する（欠落＝不明として扱う）。
///
/// 単体取得の [`find_race_card`](super::find_race_card) は不正値を `Error::Data` で明示的に
/// 失敗させるのに対し、こちらは一覧用途のため 1 行の破損で全体を落とさず縮退する（方針差は意図的）。
pub async fn find_post_times_by_date(
    pool: &PgPool,
    date: NaiveDate,
) -> Result<HashMap<RaceId, NaiveTime>> {
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
            let id = RaceId::try_from(race_id.as_str())
                .inspect_err(|e| {
                    tracing::warn!(race_id, error = %e, "race_id を解釈できず post_time 一覧から除外");
                })
                .ok()?;
            let time = NaiveTime::parse_from_str(&post_time, "%H:%M")
                .inspect_err(|e| {
                    tracing::warn!(race_id, post_time, error = %e, "post_time を HH:MM として解釈できず一覧から除外");
                })
                .ok()?;
            Some((id, time))
        })
        .collect())
}
