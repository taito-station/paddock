use utoipa::OpenApi;

use crate::error::{ErrorBody, ErrorDetail};
use crate::handler;
use crate::schema::analyze::{
    AnalyzeCandidatesResponse, CourseStatsResponse, GroupStatSchema, HorseStatsResponse,
    JockeyStatsResponse, TrainerStatsResponse,
};
use crate::schema::live::{
    LiveFlip, LiveRaceViewSchema, LiveResponse, LiveSummary, SlipLeg, SlipView,
};
use crate::schema::prediction::{
    MarkStatSchema, MarkStatsResponse, PadPredictionResponse, PredictionBetSchema,
    PredictionHorseSchema, PredictionResultSchema, PredictionSearchResponse,
    PredictionSummarySchema,
};
use crate::schema::race::{
    BoardHorseSchema, ConfusionSchema, HorseEntrySchema, HorseProbabilitySchema,
    PredictionResponse, RaceBoardResponse, RaceCardResponse, RaceListResponse, RaceSummary,
    RecommendationBet, RecommendationResponse,
};
use crate::schema::session::{
    BetInput, CreateSessionRequest, OddsRefreshResponse, RecordOutcomeRequest,
    SessionSummaryResponse, SettleReportResponse, SummaryBet,
};

/// paddock REST API（read + session write, #33 / #53）の OpenAPI ドキュメント。
/// handler の `#[utoipa::path]` と schema の `#[derive(ToSchema)]` から生成する（コードファースト）。
#[derive(OpenApi)]
#[openapi(
    info(
        title = "paddock REST API",
        version = "0.1.0",
        description = "競馬予想・分析の REST API。read 系（#33）と予想セッション write 系（#53）を提供する。",
        license(name = "Proprietary")
    ),
    paths(
        handler::race::list_races,
        handler::race::get_race_card,
        handler::race::get_prediction,
        handler::race::get_recommendations,
        handler::race::get_race_board,
        handler::analyze::analyze_horse,
        handler::analyze::analyze_horse_candidates,
        handler::analyze::analyze_jockey,
        handler::analyze::analyze_jockey_candidates,
        handler::analyze::analyze_trainer,
        handler::analyze::analyze_trainer_candidates,
        handler::analyze::analyze_course,
        handler::prediction::search_predictions,
        handler::prediction::get_prediction_detail,
        handler::prediction::prediction_mark_stats,
        handler::live::get_live,
        handler::session::create_session,
        handler::session::get_session_summary,
        handler::session::record_outcome,
        handler::session::odds_refresh,
        handler::session::results_refresh,
    ),
    components(schemas(
        RaceSummary,
        RaceListResponse,
        HorseEntrySchema,
        RaceCardResponse,
        HorseProbabilitySchema,
        PredictionResponse,
        RecommendationBet,
        RecommendationResponse,
        ConfusionSchema,
        BoardHorseSchema,
        RaceBoardResponse,
        GroupStatSchema,
        HorseStatsResponse,
        CourseStatsResponse,
        JockeyStatsResponse,
        TrainerStatsResponse,
        AnalyzeCandidatesResponse,
        PredictionSummarySchema,
        PredictionSearchResponse,
        PredictionHorseSchema,
        PredictionBetSchema,
        PredictionResultSchema,
        PadPredictionResponse,
        MarkStatSchema,
        MarkStatsResponse,
        LiveResponse,
        LiveSummary,
        LiveRaceViewSchema,
        SlipView,
        SlipLeg,
        LiveFlip,
        CreateSessionRequest,
        BetInput,
        RecordOutcomeRequest,
        SummaryBet,
        SessionSummaryResponse,
        OddsRefreshResponse,
        SettleReportResponse,
        ErrorBody,
        ErrorDetail,
    )),
    tags(
        (name = "races", description = "レース一覧 / 出馬表 / 確率推定"),
        (name = "analyze", description = "馬 / 騎手 / 調教師 / コースの成績統計"),
        (name = "predictions", description = "予想の横断検索 / 個別取得 / 印別的中率集計"),
        (name = "sessions", description = "予想セッション（作成 / 収支 / 賭け金・払戻記録 / オッズ・結果更新）"),
        (name = "live", description = "ライブEV買い目（今これを買え）"),
    )
)]
pub struct ApiDoc;
