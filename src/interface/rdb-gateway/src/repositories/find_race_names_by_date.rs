use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::RaceId;
use sqlx::PgPool;

use crate::error::Result;

/// 指定日の全レースの表示用レース名を `race_id → race_name` で返す（`race_cards` 由来、#389）。
///
/// レース一覧のヘッダに重賞・特別戦名を出すための一括取得（[`find_post_times_by_date`] と同方針）。
/// race_name 未保存（NULL）は SQL で除外する（PDF 経路・取得失敗・過去分＝正常系。warn なし）。
/// race_id として解釈できない行のみ warn を出してマップから除外する（1 行破損で全体を落とさない縮退）。
///
/// [`find_post_times_by_date`]: super::find_post_times_by_date::find_post_times_by_date
pub async fn find_race_names_by_date(
    pool: &PgPool,
    date: NaiveDate,
) -> Result<HashMap<RaceId, String>> {
    let date_str = date.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT race_id, race_name
        FROM race_cards
        WHERE date = $1
          AND race_name IS NOT NULL
        "#,
    )
    .bind(&date_str)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|(race_id, race_name)| {
            let id = RaceId::try_from(race_id.as_str())
                .inspect_err(|e| {
                    tracing::warn!(race_id, error = %e, "race_id を解釈できず race_name 一覧から除外");
                })
                .ok()?;
            Some((id, race_name))
        })
        .collect())
}
