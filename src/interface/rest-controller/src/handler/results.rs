use actix_web::{HttpResponse, web};
use chrono::NaiveDate;
use serde::Deserialize;

use paddock_use_case::ResultsInteractor;
use paddock_use_case::repository::{
    PredictSessionRepository, RaceCardRepository, RaceRepository, RaceResultRepository,
};
use paddock_use_case::result_page_fetcher::ResultPageFetcher;

use crate::error::{Error, Result};
use crate::schema::results::RefreshReportResponse;

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| Error::BadRequest(format!("invalid date '{s}' (YYYY-MM-DD): {e}")))
}

/// `?force=` クエリ（既定 false）。手動フォールバックのみ true を渡す（#381・ADR 0068）。
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct RefreshQuery {
    /// true で post_time gating を緩和し、post_time 未取得・未発走の未確定レースも取得対象にする。
    #[serde(default)]
    pub force: bool,
}

async fn run_refresh<S, R>(
    interactor: &ResultsInteractor<S, R>,
    date: NaiveDate,
    force: bool,
) -> Result<HttpResponse>
where
    S: ResultPageFetcher,
    R: RaceRepository + RaceCardRepository + PredictSessionRepository + RaceResultRepository,
{
    let report = interactor.refresh(date, force).await?;
    Ok(HttpResponse::Ok().json(RefreshReportResponse::from(report)))
}

/// 同日のレース結果（着順・確定払戻）を取り込み、セッションがあれば自動精算する（#381・冪等）。
///
/// 対象は「発走済み かつ 未確定」のレース。`?force=true` で post_time gating を緩和する（手動救済）。
#[utoipa::path(
    post,
    path = "/api/results/{date}:refresh",
    params(
        ("date" = String, Path, description = "開催日 YYYY-MM-DD"),
        RefreshQuery,
    ),
    responses(
        (status = 200, description = "取り込み＋精算レポート（pending_races に未確定が出る）", body = RefreshReportResponse),
        (status = 400, description = "日付不正", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "results",
)]
pub async fn refresh<S, R>(
    interactor: web::Data<ResultsInteractor<S, R>>,
    path: web::Path<String>,
    query: web::Query<RefreshQuery>,
) -> Result<HttpResponse>
where
    S: ResultPageFetcher + Send + Sync + 'static,
    R: RaceRepository
        + RaceCardRepository
        + PredictSessionRepository
        + RaceResultRepository
        + Send
        + Sync
        + 'static,
{
    let date = parse_date(&path.into_inner())?;
    run_refresh(interactor.get_ref(), date, query.force).await
}

/// 後方互換エイリアス `POST /api/sessions/{date}/results:refresh`（#40 の入口）。
///
/// 本フロー（`ResultsInteractor::refresh`）へ委譲し、手動フォールバックとして `force=true` で叩く。
/// レスポンスは上位互換（従来の精算集計フィールド＋着順取り込みの確定情報）。着順 upsert という
/// 副作用が加わる点が純粋な後方互換との差（ADR 0068）。
#[utoipa::path(
    post,
    path = "/api/sessions/{date}/results:refresh",
    params(("date" = String, Path, description = "開催日 YYYY-MM-DD")),
    responses(
        (status = 200, description = "取り込み＋精算レポート（pending_races に未確定が出る）", body = RefreshReportResponse),
        (status = 400, description = "日付不正", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "sessions",
)]
pub async fn session_alias_refresh<S, R>(
    interactor: web::Data<ResultsInteractor<S, R>>,
    path: web::Path<String>,
) -> Result<HttpResponse>
where
    S: ResultPageFetcher + Send + Sync + 'static,
    R: RaceRepository
        + RaceCardRepository
        + PredictSessionRepository
        + RaceResultRepository
        + Send
        + Sync
        + 'static,
{
    let date = parse_date(&path.into_inner())?;
    run_refresh(interactor.get_ref(), date, true).await
}
