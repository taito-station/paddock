use actix_web::{HttpResponse, web};
use serde::Deserialize;
use utoipa::IntoParams;

use paddock_domain::{HorseName, JockeyName, Surface, TrainerName, Venue};
use paddock_use_case::Interactor;
use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::Repository;

use crate::error::Result;
use crate::schema::analyze::{
    CourseStatsResponse, HorseStatsResponse, JockeyStatsResponse, TrainerStatsResponse,
};

/// 名前で引く分析（馬 / 騎手 / 調教師）のクエリ。
#[derive(Debug, Deserialize, IntoParams)]
pub struct NameQuery {
    /// 対象の名前（完全一致。あいまい検索は #50）。
    pub name: String,
}

/// コース分析のクエリ。
#[derive(Debug, Deserialize, IntoParams)]
pub struct CourseQuery {
    /// 開催場（英字スラッグまたは日本語）。
    pub venue: String,
    /// 距離[m]。
    pub distance: u32,
    /// 芝/ダート（`turf` / `dirt`）。
    pub surface: String,
}

/// 馬の成績統計。
#[utoipa::path(
    get,
    path = "/api/analyze/horse",
    params(NameQuery),
    responses(
        (status = 200, description = "馬の成績統計", body = HorseStatsResponse),
        (status = 400, description = "名前不正", body = crate::error::ErrorBody),
    ),
    tag = "analyze",
)]
pub async fn analyze_horse<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    query: web::Query<NameQuery>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let name = HorseName::try_from(query.name.as_str())?;
    let stats = interactor.horse_stats(&name).await?;
    Ok(HttpResponse::Ok().json(HorseStatsResponse::from(stats)))
}

/// 騎手の成績統計。
#[utoipa::path(
    get,
    path = "/api/analyze/jockey",
    params(NameQuery),
    responses(
        (status = 200, description = "騎手の成績統計", body = JockeyStatsResponse),
        (status = 400, description = "名前不正", body = crate::error::ErrorBody),
    ),
    tag = "analyze",
)]
pub async fn analyze_jockey<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    query: web::Query<NameQuery>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let name = JockeyName::try_from(query.name.as_str())?;
    let stats = interactor.jockey_stats(&name).await?;
    Ok(HttpResponse::Ok().json(JockeyStatsResponse::from(stats)))
}

/// 調教師の成績統計。
#[utoipa::path(
    get,
    path = "/api/analyze/trainer",
    params(NameQuery),
    responses(
        (status = 200, description = "調教師の成績統計", body = TrainerStatsResponse),
        (status = 400, description = "名前不正", body = crate::error::ErrorBody),
    ),
    tag = "analyze",
)]
pub async fn analyze_trainer<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    query: web::Query<NameQuery>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let name = TrainerName::try_from(query.name.as_str())?;
    let stats = interactor.trainer_stats(&name).await?;
    Ok(HttpResponse::Ok().json(TrainerStatsResponse::from(stats)))
}

/// コース（場×距離×馬場）の枠順別統計。
#[utoipa::path(
    get,
    path = "/api/analyze/course",
    params(CourseQuery),
    responses(
        (status = 200, description = "コースの枠順別統計", body = CourseStatsResponse),
        (status = 400, description = "パラメータ不正", body = crate::error::ErrorBody),
    ),
    tag = "analyze",
)]
pub async fn analyze_course<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    query: web::Query<CourseQuery>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let venue = Venue::try_from(query.venue.as_str())?;
    let surface = Surface::try_from(query.surface.as_str())?;
    let stats = interactor
        .course_stats(venue, query.distance, surface)
        .await?;
    Ok(HttpResponse::Ok().json(CourseStatsResponse::from(stats)))
}
