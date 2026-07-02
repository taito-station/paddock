use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{HorseName, RecentRun, Surface};
use sqlx::PgPool;

use super::find_finished_races_between::{ResultRow, row_to_result};
use crate::error::Result;

/// `races.date` と当該レースの surface/distance、結果カラムをまとめて受けるための行
/// （`ResultRow` を flatten で再利用、surface/distance は前走タイムの標準タイム突合用 #76）。
#[derive(sqlx::FromRow)]
struct RecentRow {
    date: String,
    surface: String,
    distance: i64,
    // 脚質（先行度）導出の元（#329 Phase1）。netkeiba 近走のみ値を持ち、pdf 側は UNION で NULL。
    corner_positions: Option<String>,
    field_size: Option<i64>,
    #[sqlx(flatten)]
    result: ResultRow,
}

/// 指定馬の `before` より前の成績を date 降順で最大 `limit` 件取得する（前走フォーム #31 用）。
///
/// pdf 確定成績(`results`)と netkeiba 近走(`horse_past_runs`)を UNION し、`date < before` で
/// バックテスト時のリークを防ぐ。同一実レースが両ソースに存在する場合は `(date, venue, race_num)`
/// 単位で **pdf を優先**して 1 件に dedup する（pdf=src_rank 0, netkeiba=1）。同点（同ソース・
/// 同一実レースの別 race_id）は `race_id` 降順で決定的に 1 件を選ぶ（前走フォームが run ごとに
/// ブレないようにするため）。
pub async fn find_recent_runs(
    pool: &PgPool,
    name: &HorseName,
    before: NaiveDate,
    limit: u32,
) -> Result<Vec<RecentRun>> {
    let before_str = before.format("%Y-%m-%d").to_string();

    let rows: Vec<RecentRow> = sqlx::query_as(
        r#"
        WITH unioned AS (
            SELECT
                races.date AS date, races.venue AS venue, races.race_num AS race_num,
                races.surface AS surface, races.distance AS distance,
                NULL AS corner_positions, NULL::integer AS field_size,
                0 AS src_rank,
                results.race_id AS race_id, results.finishing_position AS finishing_position,
                results.status AS status, results.gate_num AS gate_num,
                results.horse_num AS horse_num, results.horse_name AS horse_name,
                results.horse_id AS horse_id, results.jockey AS jockey,
                results.trainer AS trainer, results.time_seconds AS time_seconds,
                results.margin AS margin, results.odds AS odds,
                results.horse_weight AS horse_weight, results.weight_change AS weight_change,
                results.weight_carried AS weight_carried, results.popularity AS popularity
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1 AND races.date < $2 AND races.source = 'pdf'
            UNION ALL
            -- horse_past_runs は定義上 netkeiba 専用テーブルなので source 絞り込みは不要。
            SELECT
                date, venue, race_num,
                surface, distance,
                corner_positions, field_size,
                1 AS src_rank,
                race_id, finishing_position, status, gate_num, horse_num, horse_name,
                horse_id, jockey, NULL AS trainer, time_seconds, margin, odds,
                horse_weight, weight_change, weight_carried, popularity
            FROM horse_past_runs
            WHERE horse_name = $3 AND date < $4
        )
        SELECT
            u.date, u.surface, u.distance,
            u.corner_positions, u.field_size,
            u.race_id, u.finishing_position, u.status, u.gate_num, u.horse_num,
            u.horse_name, u.horse_id, u.jockey, u.trainer, u.time_seconds, u.margin,
            u.odds, u.horse_weight, u.weight_change, u.weight_carried, u.popularity
        FROM unioned u
        WHERE NOT EXISTS (
            SELECT 1 FROM unioned u2
            WHERE u2.date = u.date AND u2.venue = u.venue AND u2.race_num = u.race_num
              AND (u2.src_rank < u.src_rank
                   OR (u2.src_rank = u.src_rank AND u2.race_id > u.race_id))
        )
        ORDER BY u.date DESC, u.race_id DESC
        LIMIT $5
        "#,
    )
    .bind(name.value())
    .bind(&before_str)
    .bind(name.value())
    .bind(&before_str)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    let mut runs = Vec::with_capacity(rows.len());
    for row in rows {
        let date = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d")
            .map_err(|e| crate::Error::Data(format!("invalid race date: {e}")))?;
        runs.push(RecentRun {
            date,
            surface: Surface::try_from(row.surface.as_str())?,
            distance: row.distance as u32,
            result: row_to_result(row.result)?,
            corner_positions: row.corner_positions,
            field_size: row.field_size.map(|n| n as u32),
        });
    }
    Ok(runs)
}

/// 複数馬の `find_recent_runs` を全馬一括で取得する（#196）。各馬につき `before` より前の直近 `limit`
/// 件を date 降順で返す。per-item の UNION/dedup/ORDER をそのまま保ち、馬ごとの top-`limit` を
/// `ROW_NUMBER() OVER (PARTITION BY horse_name ORDER BY date DESC, race_id DESC)` で抽出する。
/// dedup の `NOT EXISTS` は `horse_name` でも相関させ、別馬の同日同レースが互いに dedup し合わない
/// ようにする（per-item は horse_name 固定なので不要だった条件）。返却 map は `names` の全馬を含む
/// （前走が無い馬は空 `Vec`）。per-item を各馬に呼んだ結果と順序まで一致する。
pub async fn recent_runs_batch(
    pool: &PgPool,
    names: &[HorseName],
    before: NaiveDate,
    limit: u32,
) -> Result<HashMap<HorseName, Vec<RecentRun>>> {
    let mut unique: Vec<HorseName> = Vec::new();
    for n in names {
        if !unique.contains(n) {
            unique.push(n.clone());
        }
    }
    if unique.is_empty() {
        return Ok(HashMap::new());
    }
    let name_strs: Vec<&str> = unique.iter().map(|n| n.value()).collect();
    let before_str = before.format("%Y-%m-%d").to_string();

    let rows: Vec<RecentRow> = sqlx::query_as(
        r#"
        WITH unioned AS (
            SELECT
                races.date AS date, races.venue AS venue, races.race_num AS race_num,
                races.surface AS surface, races.distance AS distance,
                NULL AS corner_positions, NULL::integer AS field_size,
                0 AS src_rank,
                results.race_id AS race_id, results.finishing_position AS finishing_position,
                results.status AS status, results.gate_num AS gate_num,
                results.horse_num AS horse_num, results.horse_name AS horse_name,
                results.horse_id AS horse_id, results.jockey AS jockey,
                results.trainer AS trainer, results.time_seconds AS time_seconds,
                results.margin AS margin, results.odds AS odds,
                results.horse_weight AS horse_weight, results.weight_change AS weight_change,
                results.weight_carried AS weight_carried, results.popularity AS popularity
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = ANY($1) AND races.date < $2 AND races.source = 'pdf'
            UNION ALL
            -- horse_past_runs は定義上 netkeiba 専用テーブルなので source 絞り込みは不要。
            SELECT
                date, venue, race_num,
                surface, distance,
                corner_positions, field_size,
                1 AS src_rank,
                race_id, finishing_position, status, gate_num, horse_num, horse_name,
                horse_id, jockey, NULL AS trainer, time_seconds, margin, odds,
                horse_weight, weight_change, weight_carried, popularity
            FROM horse_past_runs
            WHERE horse_name = ANY($3) AND date < $4
        ),
        deduped AS (
            SELECT
                u.date, u.surface, u.distance,
                u.corner_positions, u.field_size,
                u.race_id, u.finishing_position, u.status, u.gate_num, u.horse_num,
                u.horse_name, u.horse_id, u.jockey, u.trainer, u.time_seconds, u.margin,
                u.odds, u.horse_weight, u.weight_change, u.weight_carried, u.popularity,
                ROW_NUMBER() OVER (
                    PARTITION BY u.horse_name
                    ORDER BY u.date DESC, u.race_id DESC
                ) AS rn
            FROM unioned u
            WHERE NOT EXISTS (
                SELECT 1 FROM unioned u2
                WHERE u2.horse_name = u.horse_name
                  AND u2.date = u.date AND u2.venue = u.venue AND u2.race_num = u.race_num
                  AND (u2.src_rank < u.src_rank
                       OR (u2.src_rank = u.src_rank AND u2.race_id > u.race_id))
            )
        )
        SELECT
            d.date, d.surface, d.distance,
            d.corner_positions, d.field_size,
            d.race_id, d.finishing_position, d.status, d.gate_num, d.horse_num,
            d.horse_name, d.horse_id, d.jockey, d.trainer, d.time_seconds, d.margin,
            d.odds, d.horse_weight, d.weight_change, d.weight_carried, d.popularity
        FROM deduped d
        WHERE d.rn <= $5
        ORDER BY d.horse_name, d.date DESC, d.race_id DESC
        "#,
    )
    .bind(&name_strs)
    .bind(&before_str)
    .bind(&name_strs)
    .bind(&before_str)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    // 全馬を空 Vec で初期化してから行を振り分ける（前走が無い馬も map に含める）。
    let mut out: HashMap<HorseName, Vec<RecentRun>> = HashMap::with_capacity(unique.len());
    for name in &unique {
        out.insert(name.clone(), Vec::new());
    }
    for row in rows {
        let date = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d")
            .map_err(|e| crate::Error::Data(format!("invalid race date: {e}")))?;
        let run = RecentRun {
            date,
            surface: Surface::try_from(row.surface.as_str())?,
            distance: row.distance as u32,
            result: row_to_result(row.result)?,
            corner_positions: row.corner_positions,
            field_size: row.field_size.map(|n| n as u32),
        };
        // result.horse_name は既に正規化済みの `HorseName`。これを直接キーにして振り分ける
        // （unique の馬名＝同じ正規化を通っているので一致する）。
        if let Some(v) = out.get_mut(&run.result.horse_name) {
            v.push(run);
        }
    }
    Ok(out)
}
