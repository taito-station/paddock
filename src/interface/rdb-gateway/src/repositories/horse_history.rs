use paddock_domain::HorseId;
use paddock_use_case::HorsePastRun;
use paddock_use_case::paddock_race_id_from_netkeiba;
use sqlx::SqlitePool;

use crate::error::Result;

/// netkeiba 由来の近走を `horses` / `horse_past_runs` に upsert する。
///
/// pdf 確定成績(`results`)とは別テーブルに保存し、集計の二重計上・フィールドバイアスを
/// 構造的に防ぐ（#59）。`race_id` は netkeiba 12 桁から canonical paddock 形式へ変換し、
/// pdf 側 `results.race_id` と同一名前空間に揃える（`find_recent_runs` の dedup 用）。
/// `(horse_id, race_id)` 衝突時は後勝ちで上書きする（再取得＝最新値で更新する想定）。
pub async fn upsert_horse_history(
    pool: &SqlitePool,
    horse_id: &HorseId,
    runs: &[HorsePastRun],
) -> Result<()> {
    if runs.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;

    // horses マスタ: 馬名は同一馬の全 run で一致するため先頭を採用。
    sqlx::query(
        r#"
        INSERT INTO horses (horse_id, horse_name)
        VALUES ($1, $2)
        ON CONFLICT(horse_id) DO UPDATE SET horse_name = excluded.horse_name
        "#,
    )
    .bind(horse_id.value())
    .bind(runs[0].horse_name.value())
    .execute(&mut *tx)
    .await?;

    for run in runs {
        let race_id = paddock_race_id_from_netkeiba(&run.netkeiba_race_id)
            .map_err(|e| crate::Error::Data(format!("invalid netkeiba race_id: {e}")))?;
        sqlx::query(
            r#"
            INSERT INTO horse_past_runs (
                horse_id, race_id, netkeiba_race_id, date, venue, round, day, race_num,
                surface, distance, track_condition, finishing_position, status, gate_num,
                horse_num, horse_name, jockey, time_seconds, margin, odds, horse_weight,
                weight_change, weight_carried, popularity)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                    $17, $18, $19, $20, $21, $22, $23, $24)
            ON CONFLICT(horse_id, race_id) DO UPDATE SET
                netkeiba_race_id   = excluded.netkeiba_race_id,
                date               = excluded.date,
                venue              = excluded.venue,
                round              = excluded.round,
                day                = excluded.day,
                race_num           = excluded.race_num,
                surface            = excluded.surface,
                distance           = excluded.distance,
                track_condition    = excluded.track_condition,
                finishing_position = excluded.finishing_position,
                status             = excluded.status,
                gate_num           = excluded.gate_num,
                horse_num          = excluded.horse_num,
                horse_name         = excluded.horse_name,
                jockey             = excluded.jockey,
                time_seconds       = excluded.time_seconds,
                margin             = excluded.margin,
                odds               = excluded.odds,
                horse_weight       = excluded.horse_weight,
                weight_change      = excluded.weight_change,
                weight_carried     = excluded.weight_carried,
                popularity         = excluded.popularity
            "#,
        )
        .bind(horse_id.value())
        .bind(race_id.value())
        .bind(&run.netkeiba_race_id)
        .bind(run.date.format("%Y-%m-%d").to_string())
        .bind(run.venue.as_jp())
        .bind(run.round as i64)
        .bind(run.day as i64)
        .bind(run.race_num as i64)
        .bind(run.surface.as_str())
        .bind(run.distance as i64)
        .bind(run.track_condition.as_ref().map(|c| c.as_str()))
        .bind(run.finishing_position.as_ref().map(|p| p.value() as i64))
        .bind(run.status.to_string())
        .bind(run.gate_num.value() as i64)
        .bind(run.horse_num.value() as i64)
        .bind(run.horse_name.value())
        .bind(run.jockey.as_ref().map(|j| j.value().to_string()))
        .bind(run.time_seconds.as_ref().map(|t| t.value()))
        .bind(run.margin.clone())
        .bind(run.odds)
        .bind(run.horse_weight.map(|w| w as i64))
        .bind(run.weight_change.map(|w| w as i64))
        .bind(run.weight_carried)
        .bind(run.popularity.map(|p| p as i64))
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
