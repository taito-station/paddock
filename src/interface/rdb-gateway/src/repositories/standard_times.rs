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
///
/// 代表値の性質（v1 の割り切り、いずれも backtest で寄与プラスを確認した上で許容）:
/// - 基準は「全完走頭の平均タイム」で勝ち時計ではない。好走馬ほど平均より速く >0.5、凡走馬ほど <0.5 に
///   寄るため、`recent_form` の着順系 sub-signal（人気乖離・着差）と情報が一部相関する（独立性は完全でない）。
/// - 対象馬自身の過去走も母集団から除外しない（自己参照）。最小標本 5・平均のため寄与は 1/N 程度で軽微。
/// - `HAVING COUNT(*) >=` の標本数も dedup 前の行を数えるため、二重計上レースがあると閾値が緩む向きに効く。
/// - 上側の異常タイム（非現実的に小さい値）は `> 0` 以外でガードしていない。極小タイムは `time_form` で
///   1.0 に飽和し母数からは落ちない。実データのパース経路では発生しない前提（必要なら後続で上限ガード）。
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
