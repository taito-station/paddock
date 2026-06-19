use actix_web::{HttpResponse, web};
use chrono::NaiveDate;
use serde::Deserialize;
use utoipa::IntoParams;

use paddock_domain::{HorseName, Mark, Surface, Venue};
use paddock_use_case::Interactor;
use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::{MarkStatsFilter, PredictionFilter, Repository};

use crate::error::{Error, Result};
use crate::schema::prediction::{
    MarkStatSchema, MarkStatsResponse, PadPredictionResponse, PredictionSearchResponse,
    PredictionSummarySchema,
};

/// 一覧の上限件数（超過時はここに clamp する）。
const MAX_LIMIT: u32 = 200;
/// 一覧の既定件数。
const DEFAULT_LIMIT: u32 = 50;

/// `GET /api/predictions` のクエリ。全項目任意で、指定軸のみ AND 絞り込み。
#[derive(Debug, Deserialize, IntoParams)]
pub struct PredictionSearchQuery {
    /// 期間開始（`YYYY-MM-DD`。両端含む）。
    pub date_from: Option<String>,
    /// 期間終了（`YYYY-MM-DD`。両端含む）。
    pub date_to: Option<String>,
    /// 開催場（英字スラッグまたは日本語）。
    pub venue: Option<String>,
    /// 距離下限[m]。
    pub distance_min: Option<u32>,
    /// 距離上限[m]。
    pub distance_max: Option<u32>,
    /// 芝/ダート（`turf`/`dirt`）。
    pub surface: Option<String>,
    /// 馬名の部分一致（カナ正規化される）。
    pub horse_name: Option<String>,
    /// 印スラッグ（`honmei`/`taikou`/`tanana`/`renge`/`hoshi`/`chui`）。
    pub mark: Option<String>,
    /// 的中フィルタ（`true`=的中 / `false`=不的中。未指定は全件）。
    pub hit: Option<bool>,
    /// 取得件数（既定 50・上限 200）。
    pub limit: Option<u32>,
    /// オフセット（既定 0）。
    pub offset: Option<u32>,
}

/// `GET /api/predictions/stats/by-mark` のクエリ。
#[derive(Debug, Deserialize, IntoParams)]
pub struct MarkStatsQuery {
    /// 期間開始（`YYYY-MM-DD`）。
    pub date_from: Option<String>,
    /// 期間終了（`YYYY-MM-DD`）。
    pub date_to: Option<String>,
    /// 開催場（英字スラッグまたは日本語）。
    pub venue: Option<String>,
}

fn parse_date(label: &str, value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|e| Error::BadRequest(format!("invalid {label} '{value}' (YYYY-MM-DD): {e}")))
}

/// 印スラッグを `Mark` に変換する。記号（◎ 等）は API 入力として受けず slug のみ許可する。
fn parse_mark(value: &str) -> Result<Mark> {
    match Mark::from_slug(value) {
        // from_slug は記号も受理するため、slug 正準形と一致するかで slug 入力のみに限定する。
        Some(m) if m.as_slug() == value => Ok(m),
        _ => Err(Error::BadRequest(format!(
            "invalid mark '{value}' (expected one of honmei|taikou|tanana|renge|hoshi|chui)"
        ))),
    }
}

/// 予想の横断検索（一覧）。
#[utoipa::path(
    get,
    path = "/api/predictions",
    params(PredictionSearchQuery),
    responses(
        (status = 200, description = "検索一覧", body = PredictionSearchResponse),
        (status = 400, description = "クエリ不正", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "predictions",
)]
pub async fn search_predictions<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    query: web::Query<PredictionSearchQuery>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let q = query.into_inner();

    let date_from = q
        .date_from
        .as_deref()
        .map(|v| parse_date("date_from", v))
        .transpose()?;
    let date_to = q
        .date_to
        .as_deref()
        .map(|v| parse_date("date_to", v))
        .transpose()?;
    if let (Some(from), Some(to)) = (date_from, date_to)
        && from > to
    {
        return Err(Error::BadRequest(format!(
            "date_from ({from}) must be <= date_to ({to})"
        )));
    }

    if let (Some(min), Some(max)) = (q.distance_min, q.distance_max)
        && min > max
    {
        return Err(Error::BadRequest(format!(
            "distance_min ({min}) must be <= distance_max ({max})"
        )));
    }

    let venue = q.venue.as_deref().map(Venue::try_from).transpose()?;
    let surface = q.surface.as_deref().map(Surface::try_from).transpose()?;
    let mark = q.mark.as_deref().map(parse_mark).transpose()?;

    // 馬名は #50 と同じく `HorseName` でカナ正規化（全/半角カナ・濁点合成等）してから
    // 部分一致に使う。空文字は「指定なし」として扱う（30 字超など不正値は 400）。
    let horse_name = match q
        .horse_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(raw) => Some(HorseName::try_from(raw)?.value().to_string()),
        None => None,
    };

    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = q.offset.unwrap_or(0);

    let filter = PredictionFilter {
        date_from,
        date_to,
        venue,
        distance_min: q.distance_min,
        distance_max: q.distance_max,
        surface,
        horse_name,
        mark,
        hit: q.hit,
        limit,
        offset,
    };

    let result = interactor.search_predictions(filter).await?;
    let body = PredictionSearchResponse {
        total_count: result.total_count,
        limit,
        offset,
        predictions: result
            .summaries
            .into_iter()
            .map(PredictionSummarySchema::from)
            .collect(),
    };
    Ok(HttpResponse::Ok().json(body))
}

/// 個別予想（ビューア相当・全項目）。
#[utoipa::path(
    get,
    path = "/api/predictions/{prediction_id}",
    params(("prediction_id" = i64, Path, description = "予想 ID")),
    responses(
        (status = 200, description = "個別予想", body = PadPredictionResponse),
        (status = 404, description = "未存在の予想", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "predictions",
)]
pub async fn get_prediction_detail<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    path: web::Path<i64>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let prediction_id = path.into_inner();
    let prediction = interactor
        .prediction_detail(prediction_id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("prediction: {prediction_id}")))?;
    Ok(HttpResponse::Ok().json(PadPredictionResponse::from_domain(
        prediction_id,
        prediction,
    )))
}

/// 集計の入口（印別の的中率）。
#[utoipa::path(
    get,
    path = "/api/predictions/stats/by-mark",
    params(MarkStatsQuery),
    responses(
        (status = 200, description = "印別の的中率", body = MarkStatsResponse),
        (status = 400, description = "クエリ不正", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "predictions",
)]
pub async fn prediction_mark_stats<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    query: web::Query<MarkStatsQuery>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let q = query.into_inner();
    let date_from = q
        .date_from
        .as_deref()
        .map(|v| parse_date("date_from", v))
        .transpose()?;
    let date_to = q
        .date_to
        .as_deref()
        .map(|v| parse_date("date_to", v))
        .transpose()?;
    if let (Some(from), Some(to)) = (date_from, date_to)
        && from > to
    {
        return Err(Error::BadRequest(format!(
            "date_from ({from}) must be <= date_to ({to})"
        )));
    }
    let venue = q.venue.as_deref().map(Venue::try_from).transpose()?;

    let filter = MarkStatsFilter {
        date_from,
        date_to,
        venue,
    };
    let stats = interactor.prediction_mark_stats(filter).await?;
    let body = MarkStatsResponse {
        by_mark: stats.iter().map(MarkStatSchema::from).collect(),
    };
    Ok(HttpResponse::Ok().json(body))
}
