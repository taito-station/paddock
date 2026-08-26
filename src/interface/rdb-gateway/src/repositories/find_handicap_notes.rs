//! 盤の手動ハンデ精査材料（#628・提示専用）を全出走馬ぶん 1 クエリで取る。
//!
//! 既存の `horse_stats` が返す `by_surface` / `by_distance_band` / `by_venue` は**周辺分布**で、
//! 「新潟芝1000（千直）」のような**交差条件**を表せない。適性が支配的な条件では、その交差条件の
//! 実績こそが手動精査と市場の割れ目になる（2026-08-16 新潟6R 稲妻S）ため、ここで別途集計する。
//!
//! **確率推定には入れない**（ADR 0058 / 0059 で純モデルの resolution 天井は決着済み）。
//! ここで作るのは decision-support の表示材料だけで、軸・相手の選択ロジックには一切触れない。

use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{HorseName, Surface, Venue};
use paddock_use_case::repository::{
    ConditionRun, DISTANCE_EXPERIENCE_TOLERANCE_M, HandicapNoteRow,
};
use sqlx::PgPool;

use crate::error::Result;

/// dedup 済みの過去走 1 行（集計の入力）。着順ありの行だけを載せる。
#[derive(sqlx::FromRow)]
struct PastRunRow {
    horse_name: String,
    date: String,
    venue: String,
    surface: String,
    distance: i64,
    finishing_position: i64,
    race_name: Option<String>,
}

/// pdf 確定成績(`results`) と netkeiba 近走(`horse_past_runs`) を UNION し、同一実レースを
/// `(horse_name, date, venue, race_num)` 単位で 1 件に dedup して全過去走を返す。
///
/// `find_recent_runs` との違いは 3 点:
/// 1. **`LIMIT` を持たない**。条件別実績は「今回条件で何走したか」を数えるので、直近 N 走に
///    切ると答えが変わる（キャリア全体が母集団）。
/// 2. **着順ありの行だけ**を UNION に入れる（`finishing_position IS NOT NULL`）。取消・除外は
///    走っていないので「N 走」に数えない——他の stats クエリ（`horse_stats` 等）と同じ規約。
///    片方のソースだけが着順を持つ実レースは、着順を持つ側が dedup を生き残る。
/// 3. **dedup は netkeiba を優先する**（`find_recent_runs` は pdf 優先＝逆）。
///
/// ## なぜここだけ netkeiba 優先なのか（#628 の実測）
///
/// 両ソースに存在する 31,585 走を突き合わせたところ **3,503 走（11.1%）で着順が食い違い**、
/// うち 2,666 走（76%）が `pdf = netkeiba + 1` の**系統的な 1 つズレ**だった。原因は既知の
/// PDF パーサ制約（EdiF フォントで着順カラムが欠落し、以降が繰り上がる）。
/// 実例: 2025-08-10 新潟8R 驀進特別 — netkeiba はエコロジーク 1 着（1 番人気 1.9 倍）、
/// pdf は同馬 2 着で、共通する 9 頭すべてが pdf 側で +1 されていた。
///
/// この経路が出すのは**人が読む着順そのもの**なので、9 走に 1 走ズレる列を出すと
/// 手動精査の判断材料として機能しない。よってここでは着順の直接ソース（netkeiba の馬別成績）を
/// 優先する。**スコア経路（`find_recent_runs` の pdf 優先）は変えていない**ので、確率・
/// バックテストの挙動には影響しない。
const PAST_RUNS_SQL: &str = r#"
    WITH unioned AS (
        SELECT
            races.date AS date,
            races.venue AS venue,
            races.race_num AS race_num,
            races.surface AS surface,
            races.distance AS distance,
            -- pdf は着順が 1 つズレることがある（上記 11.1%）ため後順位に置く。
            1 AS src_rank,
            results.race_id AS race_id,
            results.horse_name AS horse_name,
            results.finishing_position AS finishing_position,
            -- races は race_name を持たない（PDF 経路にレース名が無い）ので NULL を埋める。
            NULL::text AS race_name
        FROM results
        INNER JOIN races AS races ON races.race_id = results.race_id
        WHERE results.horse_name = ANY($1)
          AND results.finishing_position IS NOT NULL
          AND races.source = 'pdf'
          AND ($2::text IS NULL OR races.date < $2)
        UNION ALL
        -- horse_past_runs は定義上 netkeiba 専用テーブルなので source 絞り込みは不要。
        SELECT
            date,
            venue,
            race_num,
            surface,
            distance,
            0 AS src_rank,
            race_id,
            horse_name,
            finishing_position,
            race_name
        FROM horse_past_runs
        WHERE horse_name = ANY($1)
          AND finishing_position IS NOT NULL
          AND ($2::text IS NULL OR date < $2)
    )
    SELECT
        u.horse_name,
        u.date,
        u.venue,
        u.surface,
        u.distance,
        u.finishing_position,
        u.race_name
    FROM unioned AS u
    WHERE NOT EXISTS (
        SELECT 1
        FROM unioned AS u2
        WHERE u2.horse_name = u.horse_name
          AND u2.date = u.date
          AND u2.venue = u.venue
          AND u2.race_num = u.race_num
          AND (u2.src_rank < u.src_rank
               OR (u2.src_rank = u.src_rank AND u2.race_id > u.race_id))
    )
    ORDER BY u.horse_name, u.date DESC, u.race_id DESC
"#;

/// 全出走馬の手動ハンデ精査材料を 1 クエリで取る（#628）。
///
/// `venue` / `surface` / `distance` は**今回のレース条件**。`as_of` は他 stats と同義（`date < as_of`）で、
/// 盤は当日を渡す＝確定後に自レースが自分の過去走として混ざるのを防ぐ。
/// 返却 map は `names` の全馬を含む（過去走が 1 走も無い馬も既定値のエントリで入る）。
pub async fn find_handicap_notes(
    pool: &PgPool,
    names: &[HorseName],
    venue: Venue,
    surface: Surface,
    distance: u32,
    as_of: Option<NaiveDate>,
) -> Result<HashMap<HorseName, HandicapNoteRow>> {
    let mut unique: Vec<HorseName> = Vec::new();
    for n in names {
        if !unique.contains(n) {
            unique.push(n.clone());
        }
    }
    // 全馬を既定値で初期化してから行を振り分ける（過去走 0 件の馬も map に含める＝
    // 「該当なし」と「まだ引いていない」を呼び出し側が区別できるようにする）。
    let mut out: HashMap<HorseName, HandicapNoteRow> = HashMap::with_capacity(unique.len());
    for name in &unique {
        out.insert(name.clone(), HandicapNoteRow::default());
    }
    if unique.is_empty() {
        return Ok(out);
    }

    let name_strs: Vec<&str> = unique.iter().map(|n| n.value()).collect();
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());
    let rows: Vec<PastRunRow> = sqlx::query_as(PAST_RUNS_SQL)
        .bind(&name_strs)
        .bind(&cutoff)
        .fetch_all(pool)
        .await?;

    // 場グループ（洋芝は札幌⇄函館、それ以外は自身のみ）は日本語場名で突き合わせる
    // （`races.venue` / `horse_past_runs.venue` はどちらも `Venue::as_jp()` と同じ表記）。
    let group_jp: Vec<&str> = venue.turf_group().iter().map(|v| v.as_jp()).collect();
    let surface_key = surface.as_str();
    let distance_lo = distance.saturating_sub(DISTANCE_EXPERIENCE_TOLERANCE_M);
    let distance_hi = distance.saturating_add(DISTANCE_EXPERIENCE_TOLERANCE_M);

    for row in rows {
        // 馬名は DB の生値。`HorseName` へ正規化して map のキーと突き合わせる
        // （`names` 側も同じ正規化を通っているので一致する）。
        let Ok(name) = HorseName::try_from(row.horse_name.clone()) else {
            continue;
        };
        let Some(note) = out.get_mut(&name) else {
            continue;
        };
        let Ok(date) = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d") else {
            continue;
        };
        let row_distance = row.distance.max(0) as u32;
        let same_surface = row.surface == surface_key;

        note.total_starts += 1;
        // 行は date 降順なので、最初に見た行が前走。
        if note.last_run_date.is_none_or(|d| date > d) {
            note.last_run_date = Some(date);
        }
        if same_surface {
            note.same_surface_starts += 1;
        }
        if (distance_lo..=distance_hi).contains(&row_distance) {
            note.same_distance_starts += 1;
        }

        // 条件別実績は「同芝ダ かつ 同距離（完全一致・許容幅なし）」が前提。
        // 距離を緩めると「千直 1000m」と「1200m」が混ざって適性の話でなくなる。
        if !same_surface || row_distance != distance {
            continue;
        }
        let run = ConditionRun {
            date,
            finishing_position: row.finishing_position.max(0) as u32,
            race_name: row.race_name,
        };
        if group_jp.contains(&row.venue.as_str()) {
            note.group_runs.push(run.clone());
            if row.venue == venue.as_jp() {
                note.exact_runs.push(run);
            }
        }
    }
    Ok(out)
}
