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
    RecommendationResponse,
};

/// `GET /api/races` のクエリ。
#[derive(Debug, Deserialize, IntoParams)]
pub struct RaceListQuery {
    /// 開催日（`YYYY-MM-DD`）。
    pub date: String,
}

/// 本番モデルの市場オッズブレンド係数（ADR 0027 / ADR 0031）。
/// `blend_alpha` 省略時のデフォルト。`blend_alpha=1.0` を明示すれば素モデルを参照できる。
pub(crate) const PRODUCTION_BLEND_ALPHA: f64 = 0.3;

/// クエリの `blend_alpha` を検証してハンドラが使う値に正規化する。
/// 省略（`None`）は [`PRODUCTION_BLEND_ALPHA`] にフォールバック。範囲外は `BadRequest`。
fn resolve_blend_alpha(raw: Option<f64>) -> Result<Option<f64>> {
    match raw {
        Some(a) if !(0.0..=1.0).contains(&a) => Err(Error::BadRequest(format!(
            "blend_alpha must be within [0, 1], got {a}"
        ))),
        None => Ok(Some(PRODUCTION_BLEND_ALPHA)),
        other => Ok(other), // Some(0.0..=1.0): 明示値をそのまま使う
    }
}

/// `GET /api/races/{race_id}/prediction` のクエリ。
#[derive(Debug, Deserialize, IntoParams)]
pub struct PredictionQuery {
    /// 馬場状態（`良` / `稍重` / `重` / `不良`。略記 `稍` / `不` も可）。未指定なら馬場項なし。
    pub track_condition: Option<String>,
    /// 市場オッズ（単勝）とのブレンド係数 `[0,1]`。未指定は本番ブレンド α=0.3。素モデルは `1.0` を明示。
    #[param(default = 0.3)]
    pub blend_alpha: Option<f64>,
}

/// `GET /api/races/{race_id}/recommendations` のクエリ。
#[derive(Debug, Deserialize, IntoParams)]
pub struct RecommendationQuery {
    /// このレースに配分する予算（円）。1 以上。100 円単位の買い目に配分される。
    // `minimum` は OpenAPI ドキュメント上の注記のみで、actix の Query デシリアライズは強制しない。
    // 実際の 0 弾きはハンドラ本体の `budget == 0` チェックが担う（消さないこと）。
    #[param(minimum = 1)]
    pub budget: u64,
    /// 馬場状態（`良` / `稍重` / `重` / `不良`。略記 `稍` / `不` も可）。未指定なら馬場項なし。
    pub track_condition: Option<String>,
    /// 市場オッズ（単勝）とのブレンド係数 `[0,1]`。未指定は本番ブレンド α=0.3。素モデルは `1.0` を明示（`/prediction` と同義）。
    #[param(default = 0.3)]
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
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
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
        (status = 400, description = "レース ID 不正", body = crate::error::ErrorBody),
        (status = 404, description = "未存在のレース", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
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
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
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

    let blend_alpha = resolve_blend_alpha(query.blend_alpha)?;

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

/// 買い目推奨（軸流しポートフォリオ, EV/推奨額）。保存オッズ基準。
#[utoipa::path(
    get,
    path = "/api/races/{race_id}/recommendations",
    params(
        ("race_id" = String, Path, description = "レース ID"),
        RecommendationQuery,
    ),
    responses(
        (status = 200, description = "予算内の軸流しポートフォリオ（オッズ未保存なら odds_available=false）", body = RecommendationResponse),
        (status = 400, description = "クエリ不正", body = crate::error::ErrorBody),
        (status = 404, description = "出馬表が無いレース", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "races",
)]
pub async fn get_recommendations<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    path: web::Path<String>,
    query: web::Query<RecommendationQuery>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let race_id = RaceId::try_from(path.into_inner())?;

    if query.budget == 0 {
        return Err(Error::BadRequest("budget must be >= 1".to_string()));
    }

    let blend_alpha = resolve_blend_alpha(query.blend_alpha)?;

    let track_condition = match query.track_condition.as_deref() {
        Some(s) => Some(TrackCondition::try_from(s)?),
        None => None,
    };

    let race_id_str = race_id.value().to_string();
    let body = match interactor
        .recommend_bets(&race_id, query.budget, blend_alpha, track_condition)
        .await?
    {
        Some(portfolio) => RecommendationResponse::from_portfolio(race_id_str, portfolio),
        None => RecommendationResponse::odds_unavailable(race_id_str),
    };
    Ok(HttpResponse::Ok().json(body))
}
