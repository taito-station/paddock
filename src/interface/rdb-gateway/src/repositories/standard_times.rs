use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{StandardTimes, Surface};
use sqlx::SqlitePool;

use crate::error::Result;

/// 標準タイムに採用する (surface, distance) バケツの最小標本数。これ未満の薄いバケツは
/// 代表値が不安定なため除外し、該当する前走のタイム sub-signal を落とす（#76）。暫定値。
const STANDARD_TIME_MIN_SAMPLES: i64 = 5;

#[derive(sqlx::FromRow)]
struct StandardTimeRow {
    surface: String,
    distance: i64,
    avg_time: f64,
}

/// `before`（`date < before`）より前のコーパスから (surface, distance) 別の標準タイム[秒]を
/// 集計して返す（#76）。pdf 確定成績(`results`)と netkeiba 近走(`horse_past_runs`)を UNION し、
/// 完走（`time_seconds > 0`）の平均タイムを代表値とする。`before` で as-of リークを防ぐ。
/// `time_seconds > 0` は NULL に加えて 0 秒（異常値）も母数から落とす（`TimeSeconds` は 0.0 を許容するため防御）。
///
/// median は SQLite に無いため v1 は AVG を採用する。集計用途のため pdf/netkeiba の同一実レースの
/// 重複は dedup しない（find_recent_runs と違い (date,venue,race_num) で 1 件に畳まない）。多数の行を
/// 平均するため代表値への影響は概ね軽微だが、二重計上は「netkeiba 近走を持つ馬」かつ「両ソースに在る
/// レース」に限って効くため一様ではなく、netkeiba カバレッジが特定レース種別に偏ると当該バケツ平均が
/// その向きへわずかに歪みうる（v1 の割り切り。歪みが問題化したら find_recent_runs と dedup CTE を共有する）。
/// 馬場状態（track_condition）はプールして無視する（標本確保を優先、#76 の割り切り）。
pub async fn standard_times(pool: &SqlitePool, before: NaiveDate) -> Result<StandardTimes> {
    let before_str = before.format("%Y-%m-%d").to_string();

    let rows: Vec<StandardTimeRow> = sqlx::query_as(
        r#"
        WITH t AS (
            SELECT races.surface AS surface, races.distance AS distance,
                   results.time_seconds AS time_seconds
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE races.date < ? AND races.source = 'pdf' AND results.time_seconds > 0
            UNION ALL
            -- horse_past_runs は定義上 netkeiba 専用テーブルなので source 絞り込みは不要。
            SELECT surface, distance, time_seconds
            FROM horse_past_runs
            WHERE date < ? AND time_seconds > 0
        )
        SELECT surface, distance, AVG(time_seconds) AS avg_time
        FROM t
        GROUP BY surface, distance
        HAVING COUNT(*) >= ?
        "#,
    )
    .bind(&before_str)
    .bind(&before_str)
    .bind(STANDARD_TIME_MIN_SAMPLES)
    .fetch_all(pool)
    .await?;

    let mut by_course: HashMap<(Surface, u32), f64> = HashMap::with_capacity(rows.len());
    for row in rows {
        let surface = Surface::try_from(row.surface.as_str())?;
        by_course.insert((surface, row.distance as u32), row.avg_time);
    }
    Ok(StandardTimes::new(by_course))
}
