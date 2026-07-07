use paddock_use_case::repository::LiveEvSnapshotRecord;
use serde_json::json;
use sqlx::PgPool;

use crate::error::Result;

/// ライブ EV スナップショット 1 レコードを upsert する（#346 / ADR 0064）。
///
/// 旧 `persist_live_ev.py` の upsert を Rust 化した正本。`(race_id, captured_at)` の一意制約 +
/// `ON CONFLICT DO UPDATE` で、同一サイクルの再走（cron 二重発火・手動再走）を冪等にする。
/// `captured_at` は predict-watch が 1 スイープ 1 値（UTC rfc3339 秒精度 Z 終端）で割り当てる。
///
/// `slip` / `raw` は JSONB 列。use-case を serde 非依存に保つため DTO は構造化 `Vec` で運ばれ、
/// JSON 化はこの gateway 層で行う。`slip` は `{ race_budget, legs }`（read 側 `SlipView` と同契約）、
/// `raw` は原本アーカイブ（read パスは使わないが後方互換のため列は NOT NULL）でスカラ＋slip を写す。
pub async fn save_live_ev_snapshot(pool: &PgPool, record: &LiveEvSnapshotRecord) -> Result<()> {
    let date = record.date.format("%Y-%m-%d").to_string();

    // slip 伝票（read 側 SlipView と同一形）。leg は emit 粒度「1 leg = 1 組番 = 1 点」。
    let legs: Vec<_> = record
        .legs
        .iter()
        .map(|leg| {
            json!({
                "bet_type": leg.bet_type,
                "method": leg.method,
                "axis": leg.axis,
                "combo": leg.combo,
                "points": leg.points,
                "amount": leg.amount,
            })
        })
        .collect();
    let slip = json!({
        "race_budget": record.race_budget,
        "legs": legs,
    });
    // raw = 原本アーカイブ（Python 正本の races[] 要素 1 件に相当。全スカラ＋slip）。
    let raw = json!({
        "race_id": record.race_id,
        "venue": record.venue,
        "race_no": record.race_no,
        "verdict": record.verdict,
        "roi": record.roi,
        "konsen": record.konsen,
        "axis": record.axis,
        "axis_prob": record.axis_prob,
        "axis_win_odds": record.axis_win_odds,
        "axis_place_odds_low": record.axis_place_odds_low,
        "axis_place_odds_high": record.axis_place_odds_high,
        "odds_missing": record.odds_missing,
        "slip": slip,
    });

    sqlx::query(
        r#"
        INSERT INTO live_ev_snapshots (
            date,
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
            axis_place_odds_low,
            axis_place_odds_high,
            odds_missing,
            slip,
            raw
        )
        VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16::jsonb, $17::jsonb
        )
        ON CONFLICT (race_id, captured_at) DO UPDATE SET
            date                 = excluded.date,
            venue                = excluded.venue,
            race_no              = excluded.race_no,
            post_time            = excluded.post_time,
            verdict              = excluded.verdict,
            roi                  = excluded.roi,
            konsen               = excluded.konsen,
            axis                 = excluded.axis,
            axis_prob            = excluded.axis_prob,
            axis_win_odds        = excluded.axis_win_odds,
            axis_place_odds_low  = excluded.axis_place_odds_low,
            axis_place_odds_high = excluded.axis_place_odds_high,
            odds_missing         = excluded.odds_missing,
            slip                 = excluded.slip,
            raw                  = excluded.raw
        "#,
    )
    .bind(&date)
    .bind(&record.race_id)
    .bind(&record.venue)
    .bind(record.race_no as i64)
    .bind(&record.post_time)
    .bind(&record.captured_at)
    .bind(&record.verdict)
    .bind(record.roi)
    .bind(record.konsen)
    .bind(record.axis as i64)
    .bind(record.axis_prob)
    .bind(record.axis_win_odds)
    .bind(record.axis_place_odds_low)
    .bind(record.axis_place_odds_high)
    .bind(record.odds_missing)
    // JSON は文字列で渡し SQL 側で `::jsonb` キャストする（workspace sqlx は `json` feature 無効で
    // `serde_json::Value` を直接 bind できないため。Python 正本 `lit_jsonb` と同方式）。
    .bind(slip.to_string())
    .bind(raw.to_string())
    .execute(pool)
    .await?;

    Ok(())
}
