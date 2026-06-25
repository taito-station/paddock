use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use paddock_domain::{JockeyFormRun, JockeyName};
use sqlx::PgPool;

use crate::error::Result;

/// `find_recent_runs` の騎手版（#221）。`JockeyFormRun` は着順・人気のみ運ぶ。
/// pdf 確定成績(`results`)と netkeiba 近走(`horse_past_runs`)を UNION し、`date < before` で
/// バックテスト時のリークを防ぐ。同一実レース重複は `(date, venue, race_num)` 単位で pdf 優先 dedup。
#[derive(sqlx::FromRow)]
struct JockeyFormRow {
    // results / horse_past_runs の着順・人気は bigint(INT8) カラムなので i64 で受ける。
    finishing_position: Option<i64>,
    popularity: Option<i64>,
}

fn row_to_run(row: JockeyFormRow) -> JockeyFormRun {
    JockeyFormRun {
        finishing_position: row.finishing_position.map(|v| v as u32),
        popularity: row.popularity.map(|v| v as u32),
    }
}

pub async fn find_jockey_recent_runs(
    pool: &PgPool,
    jockey: &JockeyName,
    before: NaiveDate,
    limit: u32,
) -> Result<Vec<JockeyFormRun>> {
    let before_str = before.format("%Y-%m-%d").to_string();

    let rows: Vec<JockeyFormRow> = sqlx::query_as(
        r#"
        WITH unioned AS (
            SELECT
                races.date AS date, races.venue AS venue, races.race_num AS race_num,
                results.finishing_position AS finishing_position,
                results.popularity AS popularity,
                0 AS src_rank,
                results.race_id AS race_id
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.jockey = $1 AND races.date < $2 AND races.source = 'pdf'
            UNION ALL
            -- horse_past_runs は定義上 netkeiba 専用テーブルなので source 絞り込みは不要。
            SELECT
                date, venue, race_num,
                finishing_position, popularity,
                1 AS src_rank,
                race_id
            FROM horse_past_runs
            -- $3=$1, $4=$2: sqlx は UNION の各アームで同一 $N を再利用できないため
            -- UNION netkeiba アームにも同値の騎手名・日付を別番号でバインドする。
            WHERE jockey = $3 AND date < $4
        )
        SELECT u.finishing_position, u.popularity
        FROM unioned u
        WHERE NOT EXISTS (
            SELECT 1 FROM unioned u2
            -- 単体版: $1 で騎手が 1 名固定のため全行が同一騎手。jockey 列は unioned に無いため条件不要。
            -- 同ソース内で同一 (date,venue,race_num) が複数行ある場合は race_id 降順タイブレーク
            -- （find_recent_runs.rs と同パターン: race_id 降順で決定的に 1 件を選ぶ）。
            WHERE u2.date = u.date AND u2.venue = u.venue AND u2.race_num = u.race_num
              AND (u2.src_rank < u.src_rank
                   OR (u2.src_rank = u.src_rank AND u2.race_id > u.race_id))
        )
        ORDER BY u.date DESC, u.race_id DESC
        LIMIT $5
        "#,
    )
    .bind(jockey.value())
    .bind(&before_str)
    .bind(jockey.value()) // $3=$1
    .bind(&before_str) // $4=$2
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(row_to_run).collect())
}

/// 複数騎手の `find_jockey_recent_runs` を全騎手一括で取得する（#221）。
/// dedup の NOT EXISTS は `u2.jockey = u.jockey` でも相関させ、別騎手の同日同レースが
/// 互いに dedup し合わないようにする。
pub async fn jockey_recent_runs_batch(
    pool: &PgPool,
    jockeys: &[JockeyName],
    before: NaiveDate,
    limit: u32,
) -> Result<HashMap<JockeyName, Vec<JockeyFormRun>>> {
    // seen: O(n) の Vec.contains() を避けるため HashSet で重複チェック、unique に挿入順を保持。
    let mut seen: HashSet<&JockeyName> = HashSet::new();
    let mut unique: Vec<JockeyName> = Vec::new();
    for j in jockeys {
        if seen.insert(j) {
            unique.push(j.clone());
        }
    }
    if unique.is_empty() {
        return Ok(HashMap::new());
    }
    let jockey_strs: Vec<&str> = unique.iter().map(|j| j.value()).collect();
    let before_str = before.format("%Y-%m-%d").to_string();

    #[derive(sqlx::FromRow)]
    struct BatchRow {
        // results / horse_past_runs の着順・人気は bigint(INT8) カラムなので i64 で受ける。
        finishing_position: Option<i64>,
        popularity: Option<i64>,
        jockey: String,
    }

    let rows: Vec<BatchRow> = sqlx::query_as(
        r#"
        WITH unioned AS (
            SELECT
                races.date AS date, races.venue AS venue, races.race_num AS race_num,
                results.finishing_position AS finishing_position,
                results.popularity AS popularity,
                0 AS src_rank,
                results.race_id AS race_id,
                results.jockey AS jockey
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.jockey = ANY($1) AND races.date < $2 AND races.source = 'pdf'
            UNION ALL
            -- horse_past_runs は定義上 netkeiba 専用テーブルなので source 絞り込みは不要。
            SELECT
                date, venue, race_num,
                finishing_position, popularity,
                1 AS src_rank,
                race_id,
                jockey
            FROM horse_past_runs
            -- $3=$1, $4=$2: sqlx は UNION の各アームで同一 $N を再利用できないため
            -- UNION netkeiba アームにも同値の配列・日付を別番号でバインドする。
            WHERE jockey = ANY($3) AND date < $4
        ),
        deduped AS (
            SELECT
                u.finishing_position, u.popularity, u.jockey,
                ROW_NUMBER() OVER (
                    PARTITION BY u.jockey ORDER BY u.date DESC, u.race_id DESC
                ) AS rn
            FROM unioned u
            WHERE NOT EXISTS (
                SELECT 1 FROM unioned u2
                WHERE u2.jockey = u.jockey
                  AND u2.date = u.date AND u2.venue = u.venue AND u2.race_num = u.race_num
                  -- 同一 (date,venue,race_num) で pdf(src_rank=0) が netkeiba(1) より優先。
                  -- 同ソース内で複数行がある場合は race_id 降順タイブレーク（find_recent_runs と同パターン）。
                  AND (u2.src_rank < u.src_rank
                       OR (u2.src_rank = u.src_rank AND u2.race_id > u.race_id))
            )
        )
        SELECT finishing_position, popularity, jockey
        FROM deduped
        WHERE rn <= $5
        -- rn は PARTITION 内で date DESC, race_id DESC 順に採番されるため、
        -- ORDER BY jockey, rn は騎手ごとに date 降順（単体版の ORDER BY date DESC, race_id DESC と等価）。
        ORDER BY jockey, rn
        "#,
    )
    .bind(&jockey_strs)
    .bind(&before_str)
    .bind(&jockey_strs)
    .bind(&before_str)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    // 全騎手を空 Vec で初期化してから行を振り分ける（近走が無い騎手も map に含める）。
    let mut out: HashMap<JockeyName, Vec<JockeyFormRun>> = HashMap::with_capacity(unique.len());
    for j in &unique {
        out.insert(j.clone(), Vec::new());
    }
    for row in rows {
        let name = JockeyName::try_from(row.jockey)
            .map_err(|e| crate::Error::Data(format!("invalid jockey name: {e}")))?;
        if let Some(v) = out.get_mut(&name) {
            v.push(JockeyFormRun {
                finishing_position: row.finishing_position.map(|p| p as u32),
                popularity: row.popularity.map(|p| p as u32),
            });
        }
    }
    Ok(out)
}
