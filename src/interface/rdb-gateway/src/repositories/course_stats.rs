use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{Surface, Venue};
use paddock_use_case::repository::{
    ConditionalGateStatsRow, CourseStatsRow, GATE_FIELD_BANDS, GATE_TRACK_FIRM, GATE_TRACK_OTHER,
    GateBiasCell,
};
use sqlx::PgPool;

use crate::error::Result;

use super::sql::{
    GATE_GROUPS, STATS_AGG_EXPRS, STATS_AGG_SELECT, case_from_preds, date_lt_pred,
    group_stat_from_row, or_from_preds,
};

/// `conditional_gate_stats` の集計 tuple `(starts, wins, places, shows)`。
type GateAggStat = (i64, i64, i64, i64);
/// 条件依存枠バイアスのセルキー `(track_label, field_label, gate_label)`。
type GateCellKey = (String, String, String);

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
/// 複勝ベース率を返す（#343・提示専用）。2×3×3=18 セルを **単一 GROUP BY** で一括集計する（#358）。
/// 頭数(field_size)は `results` に列が無いため CTE `race_field` で per-race COUNT を 1 回だけ結合して
/// 導出する（旧実装の行ごと相関サブクエリを排除）。`as_of` は course_stats と同義。
/// 頭数は「レースの出走頭数」なので COUNT は着順フィルタを掛けない（DNF/取消も 1 頭に数える＝レース規模）。
/// 提示側の当日頭数は `card.entries.len()`（取消含みうる）で、境界頭数(9/10・13/14)近辺の取消では帯が
/// 1 つずれ得る点は既知の近似（提示専用のため実害は限定的, measure-first）。
///
/// CASE のキー（馬場帯/頭数帯/枠群のラベル・述語）はすべて code 定数（`GATE_TRACK_PREDS`・
/// `GATE_FIELD_BANDS`・`GATE_GROUPS`）から生成し、値（venue/distance/surface/cutoff）のみ
/// プレースホルダにバインドする（course_stats と同じ二重防御・SQL インジェクション安全）。
/// 集計式は `STATS_AGG_SELECT`/`fetch_agg_grouped` と同一。GROUP BY で非空セルのみ返るため、
/// 18 セルを正準順で合成し欠損は全ゼロ（旧 COUNT(*)=0 と同一 `GroupStat`）で埋める。
pub async fn conditional_gate_stats(
    pool: &PgPool,
    venue: Venue,
    distance: u32,
    surface: Surface,
    as_of: Option<NaiveDate>,
) -> Result<ConditionalGateStatsRow> {
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());
    let date = date_lt_pred(cutoff.as_deref(), "$4");

    // CASE キー式・対象セル絞り込みの WHERE 片を code 定数から生成（SQL 安全・ドリフト無し）。
    let track_case = case_from_preds(GATE_TRACK_PREDS);
    let track_filter = or_from_preds(GATE_TRACK_PREDS);
    let gate_case = case_from_preds(GATE_GROUPS);
    let gate_filter = or_from_preds(GATE_GROUPS);
    // 頭数帯は (label, lo, hi) 形式で CASE を組む（rf.field_size を BETWEEN で帯分け）。
    // 帯は 1-99 を隙間なく被覆する code 定数（field_size は per-race COUNT で最低 1・実頭数 ≤18）
    // なので CASE は必ずいずれかの帯に当たり NULL を返さない。ラベルは SQL リテラルに素で埋めるため
    // 単一引用符を禁止（case_from_preds と同じ契約）。
    let mut field_case = String::from("CASE");
    for (label, lo, hi) in GATE_FIELD_BANDS {
        debug_assert!(
            !label.contains('\''),
            "field band label must not contain a single quote: {label:?}"
        );
        field_case.push_str(&format!(
            " WHEN rf.field_size BETWEEN {lo} AND {hi} THEN '{label}'"
        ));
    }
    field_case.push_str(" END");

    // 集計式は共通 `STATS_AGG_EXPRS`（fetch_agg_grouped と同じくキー列を前置してグループ集計）。
    // 頭数(field_size)は対象コースのレースだけに絞って COUNT する（`target_races` 経由）。全 results の
    // Seq Scan を避け、`race_id` インデックス経由で該当レース分のみ走査させる（#358 acceptance）。
    let query_str = format!(
        r#"
        WITH target_races AS (
            SELECT races.race_id
            FROM races
            WHERE races.venue = $1 AND races.distance = $2 AND races.surface = $3
              AND ({track_filter}) {date}
        ),
        race_field AS (
            SELECT results.race_id, COUNT(*) AS field_size
            FROM results
            INNER JOIN target_races ON target_races.race_id = results.race_id
            GROUP BY results.race_id
        )
        SELECT
            {track_case} AS track_key,
            {field_case} AS field_key,
            {gate_case} AS gate_key,
            {STATS_AGG_EXPRS}
        FROM results
        INNER JOIN races ON races.race_id = results.race_id
        INNER JOIN race_field AS rf ON rf.race_id = results.race_id
        WHERE results.finishing_position IS NOT NULL
          AND ({gate_filter})
        GROUP BY track_key, field_key, gate_key
        "#,
    );

    let mut query = sqlx::query_as::<_, (String, String, String, i64, i64, i64, i64)>(
        sqlx::AssertSqlSafe(query_str),
    )
    .bind(venue.as_jp())
    .bind(distance as i64)
    .bind(surface.as_str());
    if let Some(d) = &cutoff {
        query = query.bind(d);
    }
    let rows = query.fetch_all(pool).await?;

    let mut agg: HashMap<GateCellKey, GateAggStat> = HashMap::new();
    for (track, field, gate, starts, wins, places, shows) in rows {
        agg.insert((track, field, gate), (starts, wins, places, shows));
    }

    // 18 セルを正準順（旧実装のループ順）で合成。非空セルは agg から、欠損は全ゼロで埋める。
    let mut cells =
        Vec::with_capacity(GATE_TRACK_PREDS.len() * GATE_FIELD_BANDS.len() * GATE_GROUPS.len());
    for (track_label, _) in GATE_TRACK_PREDS {
        for (field_label, _, _) in GATE_FIELD_BANDS {
            for (gate_label, _) in GATE_GROUPS {
                let key = (
                    track_label.to_string(),
                    field_label.to_string(),
                    gate_label.to_string(),
                );
                let row = agg.get(&key).copied().unwrap_or((0, 0, 0, 0));
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
