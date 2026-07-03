use chrono::NaiveDate;
use paddock_use_case::repository::LiveEvSnapshot;
use sqlx::PgPool;

use crate::error::Result;

#[derive(sqlx::FromRow)]
struct LiveEvRow {
    rnk: i64,
    race_id: String,
    venue: String,
    race_no: i64,
    post_time: Option<String>,
    captured_at: String,
    verdict: String,
    roi: f64,
    konsen: bool,
    axis: i64,
    axis_prob: f64,
    axis_win_odds: Option<f64>,
    odds_missing: bool,
    slip: String,
}

/// 指定開催日の全 race について最新 2 サイクル（`rank<=2`）をフラットに返す（#260 / ADR 0064）。
///
/// `ROW_NUMBER() OVER (PARTITION BY race_id ORDER BY captured_at DESC)` で race ごとに
/// 最新（1）・直前（2）を採り、`rnk<=2` に絞る。`captured_at` は UTC rfc3339 の TEXT で辞書順＝
/// 時刻順のため、TEXT 比較のまま「最新」「2 番目に新しい」を得られる（`race_odds_snapshots` と同規約）。
/// `slip` JSONB は `::text` で JSON テキストとして取り出し、上位（rest-controller）の DTO で
/// デシリアライズする（use-case を serde 非依存に保つ）。`raw` 列は API レスポンスに使わないため
/// SELECT しない（原本は DB に保持されるが read パスでは不要）。
/// 並びは `(race_id, rnk)` 昇順で、interactor が最新/直前へグルーピングする。
pub async fn find_live_ev_by_date(pool: &PgPool, date: NaiveDate) -> Result<Vec<LiveEvSnapshot>> {
    let date_str = date.format("%Y-%m-%d").to_string();

    let rows: Vec<LiveEvRow> = sqlx::query_as(
        r#"
        SELECT
            rnk,
            race_id,
            venue,
            race_no,
            post_time,
            captured_at,
            verdict,
            roi,
            konsen,
            axis,
            axis_prob,
            axis_win_odds,
            odds_missing,
            slip
        FROM (
            SELECT
                race_id,
                venue,
                race_no,
                post_time,
                captured_at,
                verdict,
                roi,
                konsen,
                axis,
                axis_prob,
                axis_win_odds,
                odds_missing,
                slip::text AS slip,
                ROW_NUMBER() OVER (PARTITION BY race_id ORDER BY captured_at DESC) AS rnk
            FROM live_ev_snapshots
            WHERE date = $1
        ) AS ranked
        WHERE rnk <= 2
        ORDER BY race_id ASC, rnk ASC
        "#,
    )
    .bind(&date_str)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| LiveEvSnapshot {
            rank: row.rnk as u32,
            race_id: row.race_id,
            venue: row.venue,
            race_no: row.race_no as u32,
            post_time: row.post_time,
            captured_at: row.captured_at,
            verdict: row.verdict,
            roi: row.roi,
            konsen: row.konsen,
            axis: row.axis as u32,
            axis_prob: row.axis_prob,
            axis_win_odds: row.axis_win_odds,
            odds_missing: row.odds_missing,
            slip_json: row.slip,
        })
        .collect())
}
