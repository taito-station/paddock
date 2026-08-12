use chrono::NaiveDate;
use paddock_use_case::repository::LiveEvPin;
use sqlx::PgPool;

use crate::error::Result;

/// 混戦成立の最小頭数（domain `KONSEN_MIN_HORSES` と同値）。band の整合検査にのみ使う。
const KONSEN_MIN_HORSES: usize = 4;

#[derive(sqlx::FromRow)]
struct LiveEvPinRow {
    race_id: String,
    axis: i64,
    konsen: bool,
    captured_at: String,
    partners: Vec<i32>,
    konsen_band: Vec<i32>,
}

/// 指定開催日の race ごとに **`captured_at` 最古**（＝その日の初回スイープ）の買い目選定を返す（#601）。
///
/// `predict-watch` はスイープのたびに軸・相手・混戦判定を `rank_probs`（市場ブレンド α=0.2）から
/// 選び直すため、固定しないと選定がオッズに追随して動く。2 スイープ目以降はここで読んだ選定を
/// `PortfolioConfig` の固定フィールドへ渡し、オッズで動かすのは EV/ROI と金額だけにする。
///
/// `ROW_NUMBER() OVER (PARTITION BY race_id ORDER BY captured_at ASC)` で race ごとの最古を採る。
/// `captured_at` は UTC rfc3339 の TEXT で辞書順＝時刻順（`find_live_ev_by_date` と同規約）。
/// なお `find_live_ev_by_date` は**最新 2 件**を返す別用途なので混同しない。
///
/// 相手・band は `slip` JSONB から SQL 側で展開して typed に返す（use-case を serde 非依存に保つ
/// 既存方針。`find_live_ev_by_date` が `slip::text` を上位へ渡すのは API レスポンス用途のため）。
/// - **相手** = `method='nagashi'` かつ `bet_type IN ('quinella','wide')` の脚の組番から軸を除いた和集合。
///   両券種とも「軸×相手」のながしなので、片方の脚が予算端数で ¥0 になり脱落しても取りこぼさない。
/// - **band（印馬）** = `method='box'` の脚の組番の和集合。非混戦の行では box 脚が無く空配列になる
///   ＝「非混戦で固定」を意味する。
pub async fn find_live_ev_pins_by_date(pool: &PgPool, date: NaiveDate) -> Result<Vec<LiveEvPin>> {
    let date_str = date.format("%Y-%m-%d").to_string();

    let rows: Vec<LiveEvPinRow> = sqlx::query_as(
        r#"
        WITH ranked AS (
            SELECT
                race_id,
                axis,
                konsen,
                captured_at,
                slip,
                ROW_NUMBER() OVER (PARTITION BY race_id ORDER BY captured_at ASC) AS rnk
            FROM live_ev_snapshots
            WHERE date = $1
        )
        SELECT
            r.race_id AS race_id,
            r.axis AS axis,
            r.konsen AS konsen,
            r.captured_at AS captured_at,
            ARRAY(
                SELECT DISTINCT (e #>> '{}')::int
                FROM jsonb_array_elements(r.slip -> 'legs') AS leg,
                     jsonb_array_elements(leg -> 'combo') AS e
                WHERE leg ->> 'method' = 'nagashi'
                  AND leg ->> 'bet_type' IN ('quinella', 'wide')
                  AND (e #>> '{}')::int <> r.axis
                ORDER BY 1
            ) AS partners,
            ARRAY(
                SELECT DISTINCT (e #>> '{}')::int
                FROM jsonb_array_elements(r.slip -> 'legs') AS leg,
                     jsonb_array_elements(leg -> 'combo') AS e
                WHERE leg ->> 'method' = 'box'
                ORDER BY 1
            ) AS konsen_band
        FROM ranked AS r
        WHERE r.rnk = 1
        ORDER BY r.race_id ASC
        "#,
    )
    .bind(&date_str)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            // 記録済みの konsen フラグと、box 脚から復元した band の整合を検査する。
            // 現行の writer では konsen ⇔ band.len() >= 4 なので通常一致する。ずれるのは
            // 旧 writer 由来の行や手書き投入で、黙って通すと「混戦だったのに非混戦で固定する」
            // （＝配分ごと変わる）事故になるため可視化する。固定自体は band 側を採る
            // （band が買い目の実体で、フラグは派生値のため）。
            let band_konsen = row.konsen_band.len() >= KONSEN_MIN_HORSES;
            if band_konsen != row.konsen {
                tracing::warn!(
                    race_id = %row.race_id,
                    recorded_konsen = row.konsen,
                    derived_band = row.konsen_band.len(),
                    "初回スイープの konsen フラグと box 脚から復元した band が不整合。band 側を採用する"
                );
            }
            LiveEvPin {
                race_id: row.race_id,
                axis: row.axis as u32,
                partners: row.partners.into_iter().map(|n| n as u32).collect(),
                konsen_band: row.konsen_band.into_iter().map(|n| n as u32).collect(),
                captured_at: row.captured_at,
            }
        })
        .collect())
}
