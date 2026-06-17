use paddock_domain::RaceId;
use paddock_use_case::netkeiba_scraper::ResultRow;
use sqlx::PgPool;

use crate::error::Result;

/// netkeiba 結果由来の clean な値で既存 `results` 行を UPDATE する（`races` 行は触らない）。
///
/// jockey/trainer を netkeiba の略名表記に揃え、PDF 由来の馬主混入・フルネーム不一致を解消する
/// （predict の entry↔results join が噛み合うようにする）。`(race_id, horse_num)` 一致行のみを
/// 更新し、INSERT はしない（既存 566 レースの母数差し替え用途）。更新できた行数を返す。
///
/// 値カラムは `COALESCE($新値, 既存)` とし、netkeiba パースが当該セルで `None` を返した場合は
/// 既存の PDF 値を温存する（単一セル欠落で既存データを NULL 破壊しない）。`status` は netkeiba が
/// 常に値を持つ（完走/取消/中止…）ため直接上書きする。
pub async fn update_results(pool: &PgPool, race_id: &RaceId, rows: &[ResultRow]) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let mut updated = 0u64;
    for r in rows {
        let res = sqlx::query(
            r#"
            UPDATE results
            SET finishing_position = COALESCE($1, finishing_position),
                status = $2,
                jockey = COALESCE($3, jockey),
                trainer = COALESCE($4, trainer),
                time_seconds = COALESCE($5, time_seconds),
                odds = COALESCE($6, odds),
                horse_weight = COALESCE($7, horse_weight),
                weight_change = COALESCE($8, weight_change),
                weight_carried = COALESCE($9, weight_carried),
                popularity = COALESCE($10, popularity)
            WHERE race_id = $11
              AND horse_num = $12
            "#,
        )
        .bind(r.finishing_position.as_ref().map(|p| p.value() as i64))
        .bind(r.status.to_string())
        .bind(r.jockey.as_ref().map(|j| j.value().to_string()))
        .bind(r.trainer.as_ref().map(|t| t.value().to_string()))
        .bind(r.time_seconds.as_ref().map(|t| t.value()))
        .bind(r.odds)
        .bind(r.horse_weight.map(|w| w as i64))
        .bind(r.weight_change.map(|w| w as i64))
        .bind(r.weight_carried)
        .bind(r.popularity.map(|p| p as i64))
        .bind(race_id.value())
        .bind(r.horse_num.value() as i64)
        .execute(&mut *tx)
        .await?;
        updated += res.rows_affected();
    }
    tx.commit().await?;
    Ok(updated)
}
