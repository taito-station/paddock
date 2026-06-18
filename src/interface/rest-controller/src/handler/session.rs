use actix_web::{HttpResponse, web};
use chrono::NaiveDate;

use paddock_domain::RaceId;
use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::{PredictBetRecord, Repository};
use paddock_use_case::{Interactor, OddsInteractor, OddsScraper, PayoutFetcher, SettleInteractor};

use crate::error::{Error, Result};
use crate::schema::session::{
    CreateSessionRequest, OddsRefreshResponse, RecordOutcomeRequest, SessionSummaryResponse,
    SettleReportResponse,
};

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| Error::BadRequest(format!("invalid date '{s}' (YYYY-MM-DD): {e}")))
}

/// セッション新規作成。
#[utoipa::path(
    post,
    path = "/api/sessions/{date}",
    params(("date" = String, Path, description = "開催日 YYYY-MM-DD")),
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "作成したセッションのサマリ", body = SessionSummaryResponse),
        (status = 400, description = "budget 不正・日付不正", body = crate::error::ErrorBody),
        (status = 409, description = "同一開催日のセッションが既に存在", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "sessions",
)]
pub async fn create_session<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    path: web::Path<String>,
    body: web::Json<CreateSessionRequest>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let date = parse_date(&path.into_inner())?;
    let session = interactor.create_predict_session(date, body.budget).await?;
    Ok(HttpResponse::Created().json(SessionSummaryResponse::new(&session, &[])))
}

/// セッション収支サマリ + 明細。
#[utoipa::path(
    get,
    path = "/api/sessions/{date}",
    params(("date" = String, Path, description = "開催日 YYYY-MM-DD")),
    responses(
        (status = 200, description = "セッション収支 + 明細", body = SessionSummaryResponse),
        (status = 400, description = "日付不正", body = crate::error::ErrorBody),
        (status = 404, description = "未作成のセッション", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "sessions",
)]
pub async fn get_session_summary<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    path: web::Path<String>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let date = parse_date(&path.into_inner())?;
    let (session, bets) = interactor.session_summary(date).await?;
    Ok(HttpResponse::Ok().json(SessionSummaryResponse::new(&session, &bets)))
}

/// 1 レース分の賭け金・払戻を記録（残高ガード・1 トランザクション）。
#[utoipa::path(
    post,
    path = "/api/sessions/{date}/races/{race_id}/outcome",
    params(
        ("date" = String, Path, description = "開催日 YYYY-MM-DD"),
        ("race_id" = String, Path, description = "レース ID"),
    ),
    request_body = RecordOutcomeRequest,
    responses(
        (status = 200, description = "更新後のセッションサマリ", body = SessionSummaryResponse),
        (status = 400, description = "残高超過・日付/ID 不正", body = crate::error::ErrorBody),
        (status = 404, description = "未作成のセッション", body = crate::error::ErrorBody),
        (status = 409, description = "当該レースの outcome が既に記録済み", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "sessions",
)]
pub async fn record_outcome<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    path: web::Path<(String, String)>,
    body: web::Json<RecordOutcomeRequest>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let (date_str, race_id_str) = path.into_inner();
    let date = parse_date(&date_str)?;
    let race_id = RaceId::try_from(race_id_str)?;

    let bets: Vec<PredictBetRecord> = body
        .into_inner()
        .bets
        .into_iter()
        .map(|b| PredictBetRecord {
            race_id: race_id.clone(),
            bet_type: b.bet_type,
            combination: b.combination,
            stake: b.stake,
            payout: b.payout,
            ev: b.ev,
        })
        .collect();

    // record_race_outcome が更新後 session を返すので再取得せず、明細 bets のみ取り直す。
    let session = interactor.record_race_outcome(date, &race_id, bets).await?;
    let all_bets = interactor.find_predict_bets(date).await?;
    Ok(HttpResponse::Ok().json(SessionSummaryResponse::new(&session, &all_bets)))
}

/// オッズをライブ取得して保存（#51, read-through）。
#[utoipa::path(
    post,
    path = "/api/sessions/{date}/races/{race_id}/odds:refresh",
    params(
        ("date" = String, Path, description = "開催日 YYYY-MM-DD"),
        ("race_id" = String, Path, description = "レース ID"),
    ),
    responses(
        (status = 200, description = "取得結果（未取得は fetched=false・HTTP は 200）", body = OddsRefreshResponse),
        (status = 404, description = "未作成のセッション", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "sessions",
)]
pub async fn odds_refresh<R, P, F, O>(
    interactor: web::Data<Interactor<R, P, F>>,
    odds: web::Data<OddsInteractor<O, R>>,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
    O: OddsScraper + Send + Sync + 'static,
{
    let (date_str, race_id_str) = path.into_inner();
    let date = parse_date(&date_str)?;
    let race_id = RaceId::try_from(race_id_str)?;

    // セッション文脈下の操作として存在を要求する（race_odds 自体はセッション非依存だが SPA の一貫性のため）。
    if interactor.find_predict_session(date).await?.is_none() {
        return Err(Error::NotFound(format!("session for {date} not found")));
    }

    let fetched = odds.race_odds(&race_id).await?.is_some();
    Ok(HttpResponse::Ok().json(OddsRefreshResponse {
        race_id: race_id.value().to_string(),
        fetched,
    }))
}

/// 確定結果を取得して払戻を自動補完（#40, 冪等）。
#[utoipa::path(
    post,
    path = "/api/sessions/{date}/results:refresh",
    params(("date" = String, Path, description = "開催日 YYYY-MM-DD")),
    responses(
        (status = 200, description = "精算レポート（pending_races に未確定が出る）", body = SettleReportResponse),
        (status = 400, description = "日付不正", body = crate::error::ErrorBody),
        (status = 404, description = "未作成のセッション", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "sessions",
)]
pub async fn results_refresh<S, R>(
    settle: web::Data<SettleInteractor<S, R>>,
    path: web::Path<String>,
) -> Result<HttpResponse>
where
    S: PayoutFetcher + Send + Sync + 'static,
    R: Repository + 'static,
{
    let date = parse_date(&path.into_inner())?;
    let report = settle.settle_session(date).await?;
    Ok(HttpResponse::Ok().json(SettleReportResponse {
        settled_races: report.settled_races as u32,
        pending_races: report.pending_races as u32,
        voided_races: report.voided_races as u32,
        refunded_bets: report.refunded_bets as u32,
        total_bet: report.total_bet,
        total_payout: report.total_payout,
        balance: report.balance,
        roi: report.roi,
    }))
}
