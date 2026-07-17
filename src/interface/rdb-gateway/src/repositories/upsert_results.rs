use std::collections::HashMap;

use paddock_domain::{HorseEntry, RaceCard};
use paddock_use_case::netkeiba_scraper::ResultRow;
use sqlx::PgPool;

use crate::error::Result;

/// 同日取り込み（#381）: `races` 行を出馬表メタから upsert（`results.race_id → races` の FK 担保）し、
/// 着順を `results` へ upsert する。upsert した着順行数を返す。
///
/// `save_race`（PDF 取り込み用）と違い **破壊的な上書き・DELETE をしない**:
/// - `races` の `track_condition`/`weather` は INSERT で書かず（同日は結果ページから取得しない）、
///   ON CONFLICT でも触らないため既存 PDF 値を温存する（過去日を手動 refresh しても消えない）。
/// - `results` の値カラムは `COALESCE(excluded, 既存)` で、netkeiba パースが `None` のセルは既存値を温存。
/// - `delete_absent_horse_nums` を呼ばない（出馬表に無い馬番の既存着順行を消さない）。
///
/// NOT NULL の `gate_num`/`horse_name` は `ResultRow` に無いため `race_cards` エントリから補完する。
/// 出馬表に無い馬番の着順行は補完不能のためスキップする（着順を書かない）。
pub async fn upsert_results(pool: &PgPool, card: &RaceCard, rows: &[ResultRow]) -> Result<u64> {
    let entry_by_num: HashMap<u32, &HorseEntry> = card
        .entries
        .iter()
        .map(|e| (e.horse_num.value(), e))
        .collect();

    let mut tx = pool.begin().await?;

    // FK 担保用の races 行。track_condition/weather は書かない（既存 PDF 値を温存するため）。
    // source は既定 'pdf'（＝実レースのバケット。find_races_by_date の UNION 条件と整合）。
    sqlx::query(
        r#"
        INSERT INTO races (race_id, date, venue, round, day, race_num, surface, distance)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT(race_id) DO UPDATE SET
            date = excluded.date,
            venue = excluded.venue,
            round = excluded.round,
            day = excluded.day,
            race_num = excluded.race_num,
            surface = excluded.surface,
            distance = excluded.distance
        "#,
    )
    .bind(card.race_id.value())
    .bind(card.date.format("%Y-%m-%d").to_string())
    .bind(card.venue.as_jp())
    .bind(card.round as i64)
    .bind(card.day as i64)
    .bind(card.race_num as i64)
    .bind(card.surface.as_str())
    .bind(card.distance as i64)
    .execute(&mut *tx)
    .await?;

    let mut upserted = 0u64;
    for r in rows {
        let Some(entry) = entry_by_num.get(&r.horse_num.value()) else {
            tracing::warn!(
                race_id = card.race_id.value(),
                horse_num = r.horse_num.value(),
                "結果の馬番が出馬表に無く gate_num/horse_name を補完できないため着順行をスキップ"
            );
            continue;
        };
        let res = sqlx::query(
            r#"
            INSERT INTO results
                (race_id, finishing_position, status, gate_num, horse_num, horse_name,
                 jockey, trainer, time_seconds, odds, horse_weight, weight_change,
                 weight_carried, popularity)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT(race_id, horse_num) DO UPDATE SET
                finishing_position = COALESCE(excluded.finishing_position, results.finishing_position),
                status = excluded.status,
                gate_num = excluded.gate_num,
                horse_name = excluded.horse_name,
                jockey = COALESCE(excluded.jockey, results.jockey),
                trainer = COALESCE(excluded.trainer, results.trainer),
                time_seconds = COALESCE(excluded.time_seconds, results.time_seconds),
                odds = COALESCE(excluded.odds, results.odds),
                horse_weight = COALESCE(excluded.horse_weight, results.horse_weight),
                weight_change = COALESCE(excluded.weight_change, results.weight_change),
                weight_carried = COALESCE(excluded.weight_carried, results.weight_carried),
                popularity = COALESCE(excluded.popularity, results.popularity)
            "#,
        )
        .bind(card.race_id.value())
        .bind(r.finishing_position.as_ref().map(|p| p.value() as i64))
        .bind(r.status.to_string())
        .bind(entry.gate_num.value() as i64)
        .bind(r.horse_num.value() as i64)
        .bind(entry.horse_name.value())
        .bind(r.jockey.as_ref().map(|j| j.value().to_string()))
        .bind(r.trainer.as_ref().map(|t| t.value().to_string()))
        .bind(r.time_seconds.as_ref().map(|t| t.value()))
        .bind(r.odds)
        .bind(r.horse_weight.map(|w| w as i64))
        .bind(r.weight_change.map(|w| w as i64))
        .bind(r.weight_carried)
        .bind(r.popularity.map(|p| p as i64))
        .execute(&mut *tx)
        .await?;
        upserted += res.rows_affected();
    }

    tx.commit().await?;
    Ok(upserted)
}
