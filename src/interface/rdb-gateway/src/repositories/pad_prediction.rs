//! 予想（印・短評・買い目・結果）の永続化。`(date, venue, race_num)` で upsert し、
//! 馬・買い目の子行は delete→insert で冪等にする。`predict_session.rs` のトランザクション
//! 流儀に倣う。

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::{
    Mark, PadPrediction, PredictionBet, PredictionHorse, PredictionResult, Venue,
};
use sqlx::{PgPool, Row};

use crate::error::{Error, Result};

fn date_key(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// `races` / `race_cards` を `(date, venue, race_num)` で照合し race_id を解決する。
/// 見つからなければ `None`（未確定・未取込レース）。venue は日本語名で格納されている。
async fn resolve_race_id(
    pool: &PgPool,
    date_str: &str,
    venue_jp: &str,
    race_num: u32,
) -> Result<Option<String>> {
    // 成績由来の races を優先し、無ければ出馬表由来の race_cards を引く。
    // pri で優先度を付けて並べ替えることで、両テーブルに別 race_id があっても決定的に races を採る。
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT race_id FROM (
            SELECT race_id, 0 AS pri FROM races
            WHERE date = $1 AND venue = $2 AND race_num = $3
            UNION ALL
            SELECT race_id, 1 AS pri FROM race_cards
            WHERE date = $1 AND venue = $2 AND race_num = $3
        )
        ORDER BY pri ASC
        LIMIT 1
        "#,
    )
    .bind(date_str)
    .bind(venue_jp)
    .bind(race_num as i64)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0))
}

pub async fn save_pad_prediction(
    pool: &PgPool,
    prediction: &PadPrediction,
    now: DateTime<Utc>,
) -> Result<()> {
    let date_str = date_key(prediction.date);
    let venue_jp = prediction.venue.as_jp().to_string();
    let race_id = resolve_race_id(pool, &date_str, &venue_jp, prediction.race_num).await?;
    let ts = now.to_rfc3339();

    let result = prediction.result.clone().unwrap_or_default();

    let mut tx = pool.begin().await?;

    // ヘッダを upsert。created_at は ON CONFLICT 対象外で初回値を保持する。
    sqlx::query(
        r#"
        INSERT INTO predictions
            (date, venue, race_num, race_id, title, budget, strategy_note, commentary,
             finish_1, finish_2, finish_3, recovery_rate, pnl, result_note,
             created_at, updated_at)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$15)
        ON CONFLICT(date, venue, race_num) DO UPDATE SET
            -- 一度解決した race_id は、後で未解決（NULL）で再取込しても巻き戻さない。
            race_id       = COALESCE(excluded.race_id, predictions.race_id),
            title         = excluded.title,
            budget        = excluded.budget,
            strategy_note = excluded.strategy_note,
            commentary    = excluded.commentary,
            finish_1      = excluded.finish_1,
            finish_2      = excluded.finish_2,
            finish_3      = excluded.finish_3,
            recovery_rate = excluded.recovery_rate,
            pnl           = excluded.pnl,
            result_note   = excluded.result_note,
            updated_at    = excluded.updated_at
        "#,
    )
    .bind(&date_str)
    .bind(&venue_jp)
    .bind(prediction.race_num as i64)
    .bind(race_id.as_deref())
    .bind(prediction.title.as_deref())
    .bind(prediction.budget.map(|b| b as i64))
    .bind(prediction.strategy_note.as_deref())
    .bind(prediction.commentary.as_deref())
    .bind(result.finish[0].map(|n| n as i64))
    .bind(result.finish[1].map(|n| n as i64))
    .bind(result.finish[2].map(|n| n as i64))
    .bind(result.recovery_rate)
    .bind(result.pnl)
    .bind(result.note.as_deref())
    .bind(&ts)
    .execute(&mut *tx)
    .await?;

    let prediction_id: i64 = sqlx::query_scalar(
        "SELECT prediction_id FROM predictions WHERE date = $1 AND venue = $2 AND race_num = $3",
    )
    .bind(&date_str)
    .bind(&venue_jp)
    .bind(prediction.race_num as i64)
    .fetch_one(&mut *tx)
    .await?;

    // 子行は入れ替え（冪等）。
    sqlx::query("DELETE FROM prediction_horses WHERE prediction_id = $1")
        .bind(prediction_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM prediction_bets WHERE prediction_id = $1")
        .bind(prediction_id)
        .execute(&mut *tx)
        .await?;

    for h in &prediction.horses {
        sqlx::query(
            r#"
            INSERT INTO prediction_horses
                (prediction_id, horse_num, horse_name, jockey, mark,
                 win_odds, popularity, win_prob, place_prob, show_prob, comment)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
            "#,
        )
        .bind(prediction_id)
        .bind(h.horse_num as i64)
        .bind(&h.horse_name)
        .bind(h.jockey.as_deref())
        .bind(h.mark.map(|m| m.as_slug()))
        .bind(h.win_odds)
        .bind(h.popularity.map(|p| p as i64))
        .bind(h.win_prob)
        .bind(h.place_prob)
        .bind(h.show_prob)
        .bind(h.comment.as_deref())
        .execute(&mut *tx)
        .await?;
    }

    for (i, b) in prediction.bets.iter().enumerate() {
        sqlx::query(
            r#"
            INSERT INTO prediction_bets
                (prediction_id, ordinal, bet_type, combination, amount)
            VALUES ($1,$2,$3,$4,$5)
            "#,
        )
        .bind(prediction_id)
        .bind(i as i64)
        .bind(&b.bet_type)
        .bind(&b.combination)
        .bind(b.amount as i64)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// `predictions` の 1 行。
#[derive(sqlx::FromRow)]
struct PredictionHeaderRow {
    prediction_id: i64,
    date: String,
    venue: String,
    race_num: i64,
    title: Option<String>,
    budget: Option<i64>,
    strategy_note: Option<String>,
    commentary: Option<String>,
    finish_1: Option<i64>,
    finish_2: Option<i64>,
    finish_3: Option<i64>,
    recovery_rate: Option<f64>,
    pnl: Option<i64>,
    result_note: Option<String>,
}

const HEADER_COLUMNS: &str = "prediction_id, date, venue, race_num, title, budget, \
    strategy_note, commentary, finish_1, finish_2, finish_3, recovery_rate, pnl, result_note";

/// `prediction_horses` の 1 行を `PredictionHorse` に変換する（find / list 共通）。
/// `prediction_id` 列の有無に依存しないので、単一・一括どちらの SELECT でも使える。
fn row_to_horse(row: &sqlx::postgres::PgRow) -> Result<PredictionHorse> {
    let mark: Option<String> = row.try_get("mark")?;
    Ok(PredictionHorse {
        horse_num: row.try_get::<i64, _>("horse_num")? as u32,
        horse_name: row.try_get("horse_name")?,
        jockey: row.try_get("jockey")?,
        mark: mark.and_then(|s| Mark::from_slug(&s)),
        win_odds: row.try_get("win_odds")?,
        popularity: row
            .try_get::<Option<i64>, _>("popularity")?
            .map(|p| p as u32),
        win_prob: row.try_get("win_prob")?,
        place_prob: row.try_get("place_prob")?,
        show_prob: row.try_get("show_prob")?,
        comment: row.try_get("comment")?,
    })
}

async fn load_horses(pool: &PgPool, prediction_id: i64) -> Result<Vec<PredictionHorse>> {
    let rows = sqlx::query(
        r#"
        SELECT horse_num, horse_name, jockey, mark, win_odds, popularity,
               win_prob, place_prob, show_prob, comment
        FROM prediction_horses
        WHERE prediction_id = $1
        ORDER BY horse_num ASC
        "#,
    )
    .bind(prediction_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_horse).collect()
}

async fn load_bets(pool: &PgPool, prediction_id: i64) -> Result<Vec<PredictionBet>> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        r#"
        SELECT bet_type, combination, amount
        FROM prediction_bets
        WHERE prediction_id = $1
        ORDER BY ordinal ASC
        "#,
    )
    .bind(prediction_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(bet_type, combination, amount)| PredictionBet {
            bet_type,
            combination,
            amount: amount as u64,
        })
        .collect())
}

/// ヘッダ行＋子行から `PadPrediction` を組み立てる（クエリは呼び出し側で実施）。
fn build_prediction(
    h: PredictionHeaderRow,
    horses: Vec<PredictionHorse>,
    bets: Vec<PredictionBet>,
) -> Result<PadPrediction> {
    let has_result = h.finish_1.is_some()
        || h.finish_2.is_some()
        || h.finish_3.is_some()
        || h.recovery_rate.is_some()
        || h.pnl.is_some()
        || h.result_note.is_some();
    let result = has_result.then(|| PredictionResult {
        finish: [
            h.finish_1.map(|n| n as u32),
            h.finish_2.map(|n| n as u32),
            h.finish_3.map(|n| n as u32),
        ],
        recovery_rate: h.recovery_rate,
        pnl: h.pnl,
        note: h.result_note,
    });

    Ok(PadPrediction {
        date: NaiveDate::parse_from_str(&h.date, "%Y-%m-%d")
            .map_err(|e| Error::Data(format!("predictions.date {:?}: {e}", h.date)))?,
        venue: Venue::try_from(h.venue.as_str())?,
        race_num: h.race_num as u32,
        title: h.title,
        budget: h.budget.map(|b| b as u64),
        strategy_note: h.strategy_note,
        commentary: h.commentary,
        horses,
        bets,
        result,
    })
}

pub async fn find_pad_prediction(
    pool: &PgPool,
    date: NaiveDate,
    venue: Venue,
    race_num: u32,
) -> Result<Option<PadPrediction>> {
    let header: Option<PredictionHeaderRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {HEADER_COLUMNS} FROM predictions WHERE date = $1 AND venue = $2 AND race_num = $3"
    )))
    .bind(date_key(date))
    .bind(venue.as_jp())
    .bind(race_num as i64)
    .fetch_optional(pool)
    .await?;

    match header {
        Some(h) => {
            let horses = load_horses(pool, h.prediction_id).await?;
            let bets = load_bets(pool, h.prediction_id).await?;
            Ok(Some(build_prediction(h, horses, bets)?))
        }
        None => Ok(None),
    }
}

pub async fn list_pad_predictions(pool: &PgPool) -> Result<Vec<PadPrediction>> {
    let headers: Vec<PredictionHeaderRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {HEADER_COLUMNS} FROM predictions ORDER BY date ASC, venue ASC, race_num ASC"
    )))
    .fetch_all(pool)
    .await?;

    if headers.is_empty() {
        return Ok(Vec::new());
    }

    // 子行は全件まとめて取得し prediction_id でグルーピングする（N+1 回避）。
    let mut horses_by: HashMap<i64, Vec<PredictionHorse>> = HashMap::new();
    let horse_rows = sqlx::query(
        r#"
        SELECT prediction_id, horse_num, horse_name, jockey, mark, win_odds, popularity,
               win_prob, place_prob, show_prob, comment
        FROM prediction_horses
        ORDER BY prediction_id ASC, horse_num ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    for row in &horse_rows {
        let pid: i64 = row.try_get("prediction_id")?;
        horses_by.entry(pid).or_default().push(row_to_horse(row)?);
    }

    let mut bets_by: HashMap<i64, Vec<PredictionBet>> = HashMap::new();
    let bet_rows: Vec<(i64, String, String, i64)> = sqlx::query_as(
        r#"
        SELECT prediction_id, bet_type, combination, amount
        FROM prediction_bets
        ORDER BY prediction_id ASC, ordinal ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    for (pid, bet_type, combination, amount) in bet_rows {
        bets_by.entry(pid).or_default().push(PredictionBet {
            bet_type,
            combination,
            amount: amount as u64,
        });
    }

    let mut out = Vec::with_capacity(headers.len());
    for h in headers {
        let pid = h.prediction_id;
        let horses = horses_by.remove(&pid).unwrap_or_default();
        let bets = bets_by.remove(&pid).unwrap_or_default();
        out.push(build_prediction(h, horses, bets)?);
    }
    Ok(out)
}
