use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::TrainerName;
use paddock_use_case::repository::TrainerStatsRow;
use sqlx::PgPool;

use crate::error::Result;

use super::sql::{entity_stats, entity_stats_batch};

/// 調教師の成績統計（overall / 芝ダ別 / 枠順別）を返す。集計本体は `entity_stats` に共通化されており
/// `jockey_stats` と列名・Row 型以外は同型（#85）。`as_of = Some(d)` のとき `races.date < d` で
/// 集計し、バックテストのリークを防ぐ（`as_of = None` の結果は従来と一致）。
pub async fn trainer_stats(
    pool: &PgPool,
    name: &TrainerName,
    as_of: Option<NaiveDate>,
) -> Result<TrainerStatsRow> {
    let n = name.value();
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());
    let (overall, by_surface, by_gate_group) =
        entity_stats(pool, "trainer", n, cutoff.as_deref()).await?;
    Ok(TrainerStatsRow {
        trainer_name: n.to_string(),
        overall,
        by_surface,
        by_gate_group,
    })
}

/// 複数調教師の [`TrainerStatsRow`] を per-item `trainer_stats` と同値でまとめて返す（#196）。
pub async fn trainer_stats_batch(
    pool: &PgPool,
    names: &[TrainerName],
    as_of: Option<NaiveDate>,
) -> Result<HashMap<TrainerName, TrainerStatsRow>> {
    let mut unique: Vec<TrainerName> = Vec::new();
    for n in names {
        if !unique.contains(n) {
            unique.push(n.clone());
        }
    }
    if unique.is_empty() {
        return Ok(HashMap::new());
    }
    let name_strs: Vec<&str> = unique.iter().map(|n| n.value()).collect();
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());
    let stats = entity_stats_batch(pool, "trainer", &name_strs, cutoff.as_deref()).await?;

    let mut out = HashMap::with_capacity(unique.len());
    for name in &unique {
        let n = name.value();
        // entity_stats_batch は `values` の全エントリ（合成ゼロ含む）を返すため None にならない。
        let (overall, by_surface, by_gate_group) = stats
            .get(n)
            .cloned()
            .expect("entity_stats_batch returns an entry for every requested value");
        out.insert(
            name.clone(),
            TrainerStatsRow {
                trainer_name: n.to_string(),
                overall,
                by_surface,
                by_gate_group,
            },
        );
    }
    Ok(out)
}
