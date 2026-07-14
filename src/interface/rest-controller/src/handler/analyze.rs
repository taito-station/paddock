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
    AnalyzeCandidatesResponse, CourseStatsResponse, HorseStatsResponse, JockeyStatsResponse,
    TrainerStatsResponse,
};

/// 名前で引く分析（馬 / 騎手 / 調教師）のクエリ。
#[derive(Debug, Deserialize, IntoParams)]
pub struct NameQuery {
    /// 対象の名前（完全一致。部分一致候補は `/candidates`・#401）。
    pub name: String,
}

/// 部分一致で候補名を引くクエリ（#401）。
#[derive(Debug, Deserialize, IntoParams)]
pub struct CandidateQuery {
    /// 検索語（中間一致。正規化は取り込み時と共有・#50）。
    pub q: String,
}

/// 候補一覧の上限。CLI `analyze`（`CANDIDATE_LIMIT`）と揃える。超過時は `truncated=true`。
const CANDIDATE_LIMIT: u32 = 20;

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
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
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
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
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
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
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

/// 上限 +1 件取得した候補を打ち切り検出しつつ `CANDIDATE_LIMIT` 件に丸める（CLI と同セマンティクス）。
fn truncate_candidates(mut names: Vec<String>) -> AnalyzeCandidatesResponse {
    let truncated = names.len() as u32 > CANDIDATE_LIMIT;
    names.truncate(CANDIDATE_LIMIT as usize);
    AnalyzeCandidatesResponse { names, truncated }
}

/// 馬名の部分一致候補（#401）。
#[utoipa::path(
    get,
    path = "/api/analyze/horse/candidates",
    params(CandidateQuery),
    responses(
        (status = 200, description = "馬名の部分一致候補", body = AnalyzeCandidatesResponse),
        (status = 400, description = "名前不正", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "analyze",
)]
pub async fn analyze_horse_candidates<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    query: web::Query<CandidateQuery>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let needle = HorseName::try_from(query.q.as_str())?;
    let names = interactor
        .find_horse_candidates(needle.value(), CANDIDATE_LIMIT + 1)
        .await?;
    Ok(HttpResponse::Ok().json(truncate_candidates(names)))
}

/// 騎手名の部分一致候補（#401）。
#[utoipa::path(
    get,
    path = "/api/analyze/jockey/candidates",
    params(CandidateQuery),
    responses(
        (status = 200, description = "騎手名の部分一致候補", body = AnalyzeCandidatesResponse),
        (status = 400, description = "名前不正", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "analyze",
)]
pub async fn analyze_jockey_candidates<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    query: web::Query<CandidateQuery>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let needle = JockeyName::try_from(query.q.as_str())?;
    let names = interactor
        .find_jockey_candidates(needle.value(), CANDIDATE_LIMIT + 1)
        .await?;
    Ok(HttpResponse::Ok().json(truncate_candidates(names)))
}

/// 調教師名の部分一致候補（#401）。
#[utoipa::path(
    get,
    path = "/api/analyze/trainer/candidates",
    params(CandidateQuery),
    responses(
        (status = 200, description = "調教師名の部分一致候補", body = AnalyzeCandidatesResponse),
        (status = 400, description = "名前不正", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "analyze",
)]
pub async fn analyze_trainer_candidates<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    query: web::Query<CandidateQuery>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let needle = TrainerName::try_from(query.q.as_str())?;
    let names = interactor
        .find_trainer_candidates(needle.value(), CANDIDATE_LIMIT + 1)
        .await?;
    Ok(HttpResponse::Ok().json(truncate_candidates(names)))
}

/// コース（場×距離×馬場）の枠順別統計。
#[utoipa::path(
    get,
    path = "/api/analyze/course",
    params(CourseQuery),
    responses(
        (status = 200, description = "コースの枠順別統計", body = CourseStatsResponse),
        (status = 400, description = "パラメータ不正", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
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

#[cfg(test)]
mod tests {
    use super::*;

    // repository の中間一致 SQL・正規化・limit は rdb-gateway の test_find_matching_names.rs で
    // 網羅済み。ここでは #401 の handler 側で新規の「上限 +1 件 → 打ち切り検出」だけを純粋に検証する。
    fn names(n: u32) -> Vec<String> {
        (0..n).map(|i| format!("馬{i}")).collect()
    }

    #[test]
    fn over_limit_is_truncated() {
        // find_*_candidates は LIMIT+1 件返す。超過分を切り詰め truncated を立てる。
        let r = truncate_candidates(names(CANDIDATE_LIMIT + 1));
        assert_eq!(r.names.len(), CANDIDATE_LIMIT as usize);
        assert!(r.truncated);
    }

    #[test]
    fn exactly_limit_is_not_truncated() {
        let r = truncate_candidates(names(CANDIDATE_LIMIT));
        assert_eq!(r.names.len(), CANDIDATE_LIMIT as usize);
        assert!(!r.truncated);
    }

    #[test]
    fn under_limit_is_preserved() {
        let r = truncate_candidates(vec!["a".to_string(), "b".to_string()]);
        assert_eq!(r.names, vec!["a".to_string(), "b".to_string()]);
        assert!(!r.truncated);
    }
}
