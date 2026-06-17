use chrono::NaiveDate;
use paddock_domain::JockeyName;
use paddock_use_case::repository::JockeyStatsRow;
use sqlx::PgPool;

use crate::error::Result;

use super::sql::entity_stats;

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
