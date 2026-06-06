use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::RaceId;
use paddock_use_case::repository::{PredictBetRecord, PredictSessionRecord};
use sqlx::SqlitePool;

use crate::error::{Error, Result};

/// `predict_sessions` の 1 行（取得用）。
type SessionRow = (String, i64, i64, i64, i64, i64, String, String);

/// 日付を DB キー文字列に整形する単一ソース。`predict_sessions.date` と
/// `predict_bets.session_date` の突き合わせはこの形式に依存するため必ずここを通す。
fn date_key(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

const SESSION_COLUMNS: &str =
    "date, budget, balance, total_bet, total_payout, completed, created_at, updated_at";

/// セッションヘッダの upsert SQL（save_predict_session / save_race_outcome 共通）。
const UPSERT_SESSION_SQL: &str = r#"
    INSERT INTO predict_sessions
        (date, budget, balance, total_bet, total_payout, completed, created_at, updated_at)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
    ON CONFLICT(date) DO UPDATE SET
        balance      = excluded.balance,
        total_bet    = excluded.total_bet,
        total_payout = excluded.total_payout,
        completed    = excluded.completed,
        updated_at   = excluded.updated_at
"#;

/// `UPSERT_SESSION_SQL` に session の各値をバインドする。
///
/// 金額（円）は `u64` だが SQLite INTEGER は `i64`。賭け金は現実的に `i64::MAX` に
/// 達しないためキャストで安全。仮に超えても `as i64` でサイレントに負値化するだけで
/// DB は受理する点に留意（ドメイン上は起き得ない）。
fn bind_session<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    date_str: &'q str,
    session: &PredictSessionRecord,
    created_at: String,
    updated_at: String,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    query
        .bind(date_str)
        .bind(session.budget as i64)
        .bind(session.balance as i64)
        .bind(session.total_bet as i64)
        .bind(session.total_payout as i64)
        .bind(i64::from(session.completed))
        .bind(created_at)
        .bind(updated_at)
}

pub async fn find_predict_session(
    pool: &SqlitePool,
    date: NaiveDate,
) -> Result<Option<PredictSessionRecord>> {
    let date_str = date_key(date);
    let row: Option<SessionRow> = sqlx::query_as(&format!(
        "SELECT {SESSION_COLUMNS} FROM predict_sessions WHERE date = $1"
    ))
    .bind(&date_str)
    .fetch_optional(pool)
    .await?;

    let Some((date_s, budget, balance, total_bet, total_payout, completed, created_at, updated_at)) =
        row
    else {
        return Ok(None);
    };

    Ok(Some(PredictSessionRecord {
        date: parse_date(&date_s)?,
        budget: budget as u64,
        balance: balance as u64,
        total_bet: total_bet as u64,
        total_payout: total_payout as u64,
        completed: completed != 0,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    }))
}

pub async fn find_predict_bets(
    pool: &SqlitePool,
    date: NaiveDate,
) -> Result<Vec<PredictBetRecord>> {
    let date_str = date_key(date);
    let rows: Vec<(String, String, String, i64, i64, f64)> = sqlx::query_as(
        r#"
        SELECT race_id, bet_type, combination, stake, payout, ev
        FROM predict_bets
        WHERE session_date = $1
        ORDER BY bet_id ASC
        "#,
    )
    .bind(&date_str)
    .fetch_all(pool)
    .await?;

    let mut bets = Vec::with_capacity(rows.len());
    for (race_id, bet_type, combination, stake, payout, ev) in rows {
        bets.push(PredictBetRecord {
            race_id: RaceId::try_from(race_id.as_str())?,
            bet_type,
            combination,
            stake: stake as u64,
            payout: payout as u64,
            ev,
        });
    }
    Ok(bets)
}

/// セッションのヘッダのみを upsert する（新規作成・完了マーク用）。
///
/// 新規セッションは必ずこの関数で先にヘッダを作成してから `save_race_outcome` を呼ぶ前提。
/// `created_at` は `ON CONFLICT DO UPDATE` の対象外なので、初回作成時の値が保持される。
pub async fn save_predict_session(pool: &SqlitePool, session: &PredictSessionRecord) -> Result<()> {
    let date_str = date_key(session.date);
    bind_session(
        sqlx::query(UPSERT_SESSION_SQL),
        &date_str,
        session,
        session.created_at.to_rfc3339(),
        session.updated_at.to_rfc3339(),
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// セッション upsert と当該レースの買い目追記を 1 トランザクションで行う。
pub async fn save_race_outcome(
    pool: &SqlitePool,
    session: &PredictSessionRecord,
    race_id: &RaceId,
    bets: &[PredictBetRecord],
) -> Result<()> {
    let date_str = date_key(session.date);
    let mut tx = pool.begin().await?;

    bind_session(
        sqlx::query(UPSERT_SESSION_SQL),
        &date_str,
        session,
        session.created_at.to_rfc3339(),
        session.updated_at.to_rfc3339(),
    )
    .execute(&mut *tx)
    .await?;

    // 買い目はレース確定と同時に記録されるため、created_at はそのレース確定時刻
    // （= session.updated_at）を用いる。
    let created_at = session.updated_at.to_rfc3339();
    for bet in bets {
        sqlx::query(
            r#"
            INSERT INTO predict_bets
                (session_date, race_id, bet_type, combination, stake, payout, ev, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(&date_str)
        .bind(race_id.value())
        .bind(&bet.bet_type)
        .bind(&bet.combination)
        .bind(bet.stake as i64)
        .bind(bet.payout as i64)
        .bind(bet.ev)
        .bind(&created_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| Error::Data(format!("predict_sessions.date {s:?}: {e}")))
}

fn parse_dt(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Data(format!("predict_sessions timestamp {s:?}: {e}")))
}
