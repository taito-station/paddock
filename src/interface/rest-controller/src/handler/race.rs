use actix_web::{HttpResponse, web};
use chrono::NaiveDate;
use serde::Deserialize;
use utoipa::IntoParams;

use paddock_domain::{RaceId, TrackCondition};
use paddock_use_case::Interactor;
use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::Repository;

use crate::error::{Error, Result};
use crate::schema::race::{
    HorseProbabilitySchema, PredictionResponse, RaceCardResponse, RaceListResponse, RaceSummary,
};

/// `GET /api/races` のクエリ。
#[derive(Debug, Deserialize, IntoParams)]
pub struct RaceListQuery {
    /// 開催日（`YYYY-MM-DD`）。
    pub date: String,
}

/// `GET /api/races/{race_id}/prediction` のクエリ。
#[derive(Debug, Deserialize, IntoParams)]
pub struct PredictionQuery {
    /// 馬場状態（`良` / `稍重` / `重` / `不良`）。未指定なら馬場項なし。
    pub track_condition: Option<String>,
    /// 市場オッズ（単勝）とのブレンド係数 `[0,1]`。未指定はモデルのみ。
    pub blend_alpha: Option<f64>,
}

/// レース一覧（指定日）。
#[utoipa::path(
    get,
    path = "/api/races",
    params(RaceListQuery),
    responses(
        (status = 200, description = "指定日のレース一覧", body = RaceListResponse),
        (status = 400, description = "日付フォーマット不正", body = crate::error::ErrorBody),
    ),
    tag = "races",
)]
pub async fn list_races<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    query: web::Query<RaceListQuery>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let date = NaiveDate::parse_from_str(&query.date, "%Y-%m-%d").map_err(|e| {
        Error::BadRequest(format!("invalid date '{}' (YYYY-MM-DD): {e}", query.date))
    })?;
    let races = interactor.races_by_date(date).await?;
    let body = RaceListResponse {
        date,
        races: races.iter().map(RaceSummary::from).collect(),
    };
    Ok(HttpResponse::Ok().json(body))
}

/// 出馬表（race card）。
#[utoipa::path(
    get,
    path = "/api/races/{race_id}",
    params(("race_id" = String, Path, description = "レース ID")),
    responses(
        (status = 200, description = "出馬表", body = RaceCardResponse),
        (status = 404, description = "未存在のレース", body = crate::error::ErrorBody),
    ),
    tag = "races",
)]
pub async fn get_race_card<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    path: web::Path<String>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let race_id = RaceId::try_from(path.into_inner())?;
    let card = interactor
        .race_card(&race_id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("race card: {}", race_id.value())))?;
    Ok(HttpResponse::Ok().json(RaceCardResponse::from(card)))
}

/// 確率推定（win/place/show）。
#[utoipa::path(
    get,
    path = "/api/races/{race_id}/prediction",
    params(
        ("race_id" = String, Path, description = "レース ID"),
        PredictionQuery,
    ),
    responses(
        (status = 200, description = "馬ごとの確率", body = PredictionResponse),
        (status = 400, description = "クエリ不正", body = crate::error::ErrorBody),
        (status = 404, description = "出馬表が無いレース", body = crate::error::ErrorBody),
    ),
    tag = "races",
)]
pub async fn get_prediction<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    path: web::Path<String>,
    query: web::Query<PredictionQuery>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let race_id = RaceId::try_from(path.into_inner())?;

    let blend_alpha = match query.blend_alpha {
        Some(a) if !(0.0..=1.0).contains(&a) => {
            return Err(Error::BadRequest(format!(
                "blend_alpha must be within [0, 1], got {a}"
            )));
        }
        other => other,
    };

    let track_condition = match query.track_condition.as_deref() {
        Some(s) => Some(TrackCondition::try_from(s)?),
        None => None,
    };

    let probs = interactor
        .predict_race(&race_id, blend_alpha, track_condition)
        .await?;
    let body = PredictionResponse {
        race_id: race_id.value().to_string(),
        probabilities: probs.iter().map(HorseProbabilitySchema::from).collect(),
    };
    Ok(HttpResponse::Ok().json(body))
}
