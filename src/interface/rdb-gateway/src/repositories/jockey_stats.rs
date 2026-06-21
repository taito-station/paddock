use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::JockeyName;
use paddock_use_case::repository::JockeyStatsRow;
use sqlx::PgPool;

use crate::error::Result;

use super::sql::{entity_stats, entity_stats_batch};

/// 騎手の成績統計（overall / 芝ダ別 / 枠順別）を返す。集計本体は `entity_stats` に共通化されており
/// `trainer_stats` と列名・Row 型以外は同型（#85）。`as_of = Some(d)` のとき `races.date < d` で
/// 集計し、バックテストのリークを防ぐ（`as_of = None` の結果は従来と一致）。
pub async fn jockey_stats(
    pool: &PgPool,
    name: &JockeyName,
    as_of: Option<NaiveDate>,
) -> Result<JockeyStatsRow> {
    let n = name.value();
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());
    let (overall, by_surface, by_gate_group) =
        entity_stats(pool, "jockey", n, cutoff.as_deref()).await?;
    Ok(JockeyStatsRow {
        jockey_name: n.to_string(),
        overall,
        by_surface,
        by_gate_group,
    })
}

/// 複数騎手の [`JockeyStatsRow`] を per-item `jockey_stats` と同値でまとめて返す（#196）。
pub async fn jockey_stats_batch(
    pool: &PgPool,
    names: &[JockeyName],
    as_of: Option<NaiveDate>,
) -> Result<HashMap<JockeyName, JockeyStatsRow>> {
    let mut unique: Vec<JockeyName> = Vec::new();
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
    let stats = entity_stats_batch(pool, "jockey", &name_strs, cutoff.as_deref()).await?;

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
            JockeyStatsRow {
                jockey_name: n.to_string(),
                overall,
                by_surface,
                by_gate_group,
            },
        );
    }
    Ok(out)
}
