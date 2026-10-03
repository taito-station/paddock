use std::collections::HashMap;

use paddock_domain::{HorseEntry, RaceCard, RaceId, ResultStatus};
use paddock_use_case::netkeiba_scraper::ResultRow;
use sqlx::PgPool;

use crate::error::{Error, Result};

/// 結果行の補完（#742）で照合に使う既存行の最小集合。
#[derive(Debug, Clone, PartialEq)]
pub struct ExistingResultRow {
    pub horse_num: u32,
    pub horse_name: String,
    pub status: ResultStatus,
    pub finishing_position: Option<u32>,
}

/// レースの既存 `results` 行（馬番・馬名・status・着順）を馬番順に読む。
pub async fn find_result_rows_for_fill(
    pool: &PgPool,
    race_id: &RaceId,
) -> Result<Vec<ExistingResultRow>> {
    let rows: Vec<(i64, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT horse_num, horse_name, status, finishing_position FROM results \
         WHERE race_id = $1 ORDER BY horse_num",
    )
    .bind(race_id.value())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(n, horse_name, status, pos)| {
            Ok(ExistingResultRow {
                horse_num: n as u32,
                horse_name,
                status: ResultStatus::try_from(status.as_str())?,
                finishing_position: pos.map(|p| p as u32),
            })
        })
        .collect()
}

/// 結果行の補完（#742）: **既存行には一切触れず**、無い馬番の行だけを INSERT する。INSERT した馬番を返す。
///
/// [`super::upsert_results::upsert_results`] と違い:
/// - `ON CONFLICT DO NOTHING` で、既存行（修正済みの馬名・status など）を上書きしない。
/// - `races` 行は書かない。無ければ何も書かずにエラーにする（race メタは別経路の責務）。
///
/// NOT NULL の `gate_num`/`horse_name` は出馬表から入れる。出馬表に無い馬番は補完できないので入れない。
/// `source` は既定の `'pdf'`（実レースのバケット）、`horse_id`・`margin` は NULL のまま。
/// 1 レースを 1 トランザクションで処理する。
pub async fn insert_missing_results(
    pool: &PgPool,
    card: &RaceCard,
    rows: &[ResultRow],
) -> Result<Vec<u32>> {
    let entry_by_num: HashMap<u32, &HorseEntry> = card
        .entries
        .iter()
        .map(|e| (e.horse_num.value(), e))
        .collect();

    let mut tx = pool.begin().await?;

    let race: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM races WHERE race_id = $1")
        .bind(card.race_id.value())
        .fetch_optional(&mut *tx)
        .await?;
    if race.is_none() {
        return Err(Error::NotFound(format!(
            "races 行がありません: {}",
            card.race_id.value()
        )));
    }

    let mut inserted = Vec::new();
    for r in rows {
        let Some(entry) = entry_by_num.get(&r.horse_num.value()) else {
            tracing::warn!(
                race_id = card.race_id.value(),
                horse_num = r.horse_num.value(),
                "結果の馬番が出馬表に無く gate_num/horse_name を補完できないため入れない"
            );
            continue;
        };
        // 列と bind は upsert_results と同じ（違うのは ON CONFLICT 句だけ）。列を変えたら upsert_results.rs も直す
        let num: Option<(i64,)> = sqlx::query_as(
            r#"
            INSERT INTO results
                (race_id, finishing_position, status, gate_num, horse_num, horse_name,
                 jockey, trainer, time_seconds, odds, horse_weight, weight_change,
                 weight_carried, popularity)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT(race_id, horse_num) DO NOTHING
            RETURNING horse_num
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
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((n,)) = num {
            inserted.push(n as u32);
        }
    }

    tx.commit().await?;
    Ok(inserted)
}

/// 接続先の DB 名（`current_database()`）。隔離 DB と共有 DB の取り違えを防ぐ照合に使う。
pub async fn current_database(pool: &PgPool) -> Result<String> {
    let (name,): (String,) = sqlx::query_as("SELECT current_database()")
        .fetch_one(pool)
        .await?;
    Ok(name)
}
