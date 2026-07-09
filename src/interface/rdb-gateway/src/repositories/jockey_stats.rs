use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::JockeyName;
use paddock_use_case::repository::JockeyStatsRow;
use sqlx::PgPool;

use crate::error::Result;

use super::sql::{
    DISTANCE_BAND_PREDS, case_from_preds, dynamic_group_stats, dynamic_group_stats_batch,
    entity_stats, entity_stats_batch,
};

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
    // #350 相性 factor: 騎手×競馬場（venue）・騎手×距離帯。venue は動的キー、距離帯は固定 CASE。
    let by_venue = dynamic_group_stats(pool, "jockey", n, "races.venue", cutoff.as_deref()).await?;
    let dist_case = case_from_preds(DISTANCE_BAND_PREDS);
    let by_distance_band =
        dynamic_group_stats(pool, "jockey", n, &dist_case, cutoff.as_deref()).await?;
    Ok(JockeyStatsRow {
        jockey_name: n.to_string(),
        overall,
        by_surface,
        by_gate_group,
        by_venue,
        by_distance_band,
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
    // #350 相性 factor（バッチ版）。venue/距離帯とも動的キー GROUP BY で全騎手一括集計する。
    // 結果に現れない騎手は空 Vec（該当実績なし＝factor None）で補完する。
    let venue_map =
        dynamic_group_stats_batch(pool, "jockey", &name_strs, "races.venue", cutoff.as_deref())
            .await?;
    let dist_case = case_from_preds(DISTANCE_BAND_PREDS);
    let distance_map =
        dynamic_group_stats_batch(pool, "jockey", &name_strs, &dist_case, cutoff.as_deref())
            .await?;

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
                by_venue: venue_map.get(n).cloned().unwrap_or_default(),
                by_distance_band: distance_map.get(n).cloned().unwrap_or_default(),
            },
        );
    }
    Ok(out)
}
