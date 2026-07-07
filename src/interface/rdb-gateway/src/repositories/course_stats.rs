use chrono::NaiveDate;
use paddock_domain::{Surface, Venue};
use paddock_use_case::repository::{
    ConditionalGateStatsRow, CourseStatsRow, GATE_FIELD_BANDS, GATE_TRACK_FIRM, GATE_TRACK_OTHER,
    GateBiasCell,
};
use sqlx::PgPool;

use crate::error::Result;

use super::sql::{GATE_GROUPS, STATS_AGG_SELECT, date_lt_pred, group_stat_from_row};

/// 馬場 2 値ラベルと SQL 述語の対応。ラベルは use-case の単一真実源（`GATE_TRACK_FIRM`/`OTHER`）を
/// 使い、述語は固定リテラルで埋め込む（`GATE_GROUPS` と同じくユーザー入力でなく安全, #343）。
const GATE_TRACK_PREDS: &[(&str, &str)] = &[
    (GATE_TRACK_FIRM, "races.track_condition = '良'"),
    (
        GATE_TRACK_OTHER,
        "races.track_condition IN ('稍重','重','不良')",
    ),
];

/// コース（場×距離×馬場）の枠順別成績を返す。集計本体・枠順グループは stats 共通ヘルパを使う（#85）。
/// `as_of = Some(d)` のとき `races.date < d` を付与する（バックテストのリーク防止）。
/// course_stats は既に `races` を JOIN しているため述語追加のみでよい。
pub async fn course_stats(
    pool: &PgPool,
    venue: Venue,
    distance: u32,
    surface: Surface,
    as_of: Option<NaiveDate>,
) -> Result<CourseStatsRow> {
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());
    // course は venue/distance/surface の多列バインド（distance は整数）で `entity_stats`/`fetch_agg`
    // の単一文字列値パターンに乗らないため、共通定数（STATS_AGG_SELECT/GATE_GROUPS）のみ使い
    // バインドは手書きする。
    let date = date_lt_pred(cutoff.as_deref(), "$4");
    let mut by_gate_group = Vec::with_capacity(GATE_GROUPS.len());
    for (label, predicate) in GATE_GROUPS {
        let q = format!(
            "{STATS_AGG_SELECT} WHERE races.venue = $1 AND races.distance = $2 \
             AND races.surface = $3 AND results.finishing_position IS NOT NULL \
             AND {predicate} {date}"
        );
        let mut query = sqlx::query_as(sqlx::AssertSqlSafe(q))
            .bind(venue.as_jp())
            .bind(distance as i64)
            .bind(surface.as_str());
        if let Some(d) = &cutoff {
            query = query.bind(d);
        }
        let row: (i64, i64, i64, i64) = query.fetch_one(pool).await?;
        by_gate_group.push(group_stat_from_row(label, row));
    }
    Ok(CourseStatsRow {
        venue: venue.as_jp().to_string(),
        distance,
        surface: surface.as_str().to_string(),
        by_gate_group,
    })
}

/// コース（場×距離×馬場）の「馬場状態(良/非良) × 頭数帯(多/中/少) × 枠群」条件依存枠バイアスの
/// 複勝ベース率を返す（#343・提示専用）。2×3×3=18 セルを個別集計する。頭数(field_size)は `results` の
/// race 単位 COUNT を相関サブクエリで導出する（`results` に頭数列は無い）。`as_of` は course_stats と同義。
/// 頭数は「レースの出走頭数」なので COUNT は着順フィルタを掛けない（DNF/取消も 1 頭に数える＝レース規模）。
/// 提示側の当日頭数は `card.entries.len()`（取消含みうる）で、境界頭数(9/10・13/14)近辺の取消では帯が
/// 1 つずれ得る点は既知の近似（提示専用のため実害は限定的, measure-first）。
///
/// 述語（馬場・頭数帯・枠）はいずれも固定リテラルの code 定数で、値（venue/distance/surface/cutoff）
/// のみプレースホルダにバインドする（course_stats と同じ二重防御・SQL インジェクション安全）。
pub async fn conditional_gate_stats(
    pool: &PgPool,
    venue: Venue,
    distance: u32,
    surface: Surface,
    as_of: Option<NaiveDate>,
) -> Result<ConditionalGateStatsRow> {
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());
    let date = date_lt_pred(cutoff.as_deref(), "$4");
    let mut cells =
        Vec::with_capacity(GATE_TRACK_PREDS.len() * GATE_FIELD_BANDS.len() * GATE_GROUPS.len());
    for (track_label, track_pred) in GATE_TRACK_PREDS {
        for (field_label, lo, hi) in GATE_FIELD_BANDS {
            for (gate_label, gate_pred) in GATE_GROUPS {
                let q = format!(
                    "{STATS_AGG_SELECT} WHERE races.venue = $1 AND races.distance = $2 \
                     AND races.surface = $3 AND results.finishing_position IS NOT NULL \
                     AND {track_pred} AND {gate_pred} \
                     AND (SELECT COUNT(*) FROM results AS r2 WHERE r2.race_id = results.race_id) \
                     BETWEEN {lo} AND {hi} {date}"
                );
                let mut query = sqlx::query_as(sqlx::AssertSqlSafe(q))
                    .bind(venue.as_jp())
                    .bind(distance as i64)
                    .bind(surface.as_str());
                if let Some(d) = &cutoff {
                    query = query.bind(d);
                }
                let row: (i64, i64, i64, i64) = query.fetch_one(pool).await?;
                cells.push(GateBiasCell {
                    track_label: track_label.to_string(),
                    field_label: field_label.to_string(),
                    gate_label: gate_label.to_string(),
                    stat: group_stat_from_row(gate_label, row),
                });
            }
        }
    }
    Ok(ConditionalGateStatsRow { cells })
}
