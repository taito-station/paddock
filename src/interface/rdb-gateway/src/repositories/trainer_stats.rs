use chrono::NaiveDate;
use paddock_domain::TrainerName;
use paddock_use_case::repository::TrainerStatsRow;
use sqlx::SqlitePool;

use crate::error::Result;

use super::sql::entity_stats;

/// 調教師の成績統計（overall / 芝ダ別 / 枠順別）を返す。集計本体は `entity_stats` に共通化されており
/// `jockey_stats` と列名・Row 型以外は同型（#85）。`as_of = Some(d)` のとき `races.date < d` で
/// 集計し、バックテストのリークを防ぐ（`as_of = None` の結果は従来と一致）。
pub async fn trainer_stats(
    pool: &SqlitePool,
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
