use utoipa::OpenApi;

use crate::error::{ErrorBody, ErrorDetail};
use crate::handler;
use crate::schema::analyze::{
    CourseStatsResponse, GroupStatSchema, HorseStatsResponse, JockeyStatsResponse,
    TrainerStatsResponse,
};
use crate::schema::race::{
    HorseEntrySchema, HorseProbabilitySchema, PredictionResponse, RaceCardResponse,
    RaceListResponse, RaceSummary,
};

/// paddock REST API（read 基盤, #33）の OpenAPI ドキュメント。
/// handler の `#[utoipa::path]` と schema の `#[derive(ToSchema)]` から生成する（コードファースト）。
#[derive(OpenApi)]
#[openapi(
    info(
        title = "paddock REST API (read)",
        version = "0.1.0",
        description = "競馬予想・分析の read 系 REST API（#33）。確率推定・出馬表・レース一覧・分析統計を提供する。",
        license(name = "Proprietary")
    ),
    paths(
        handler::race::list_races,
        handler::race::get_race_card,
        handler::race::get_prediction,
        handler::analyze::analyze_horse,
        handler::analyze::analyze_jockey,
        handler::analyze::analyze_trainer,
        handler::analyze::analyze_course,
    ),
    components(schemas(
        RaceSummary,
        RaceListResponse,
        HorseEntrySchema,
        RaceCardResponse,
        HorseProbabilitySchema,
        PredictionResponse,
        GroupStatSchema,
        HorseStatsResponse,
        CourseStatsResponse,
        JockeyStatsResponse,
        TrainerStatsResponse,
        ErrorBody,
        ErrorDetail,
    )),
    tags(
        (name = "races", description = "レース一覧 / 出馬表 / 確率推定"),
        (name = "analyze", description = "馬 / 騎手 / 調教師 / コースの成績統計"),
    )
)]
pub struct ApiDoc;
