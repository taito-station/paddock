pub mod dto;
pub mod entry_parser;
pub mod error;
pub mod interactor;
pub mod netkeiba_race_id;
pub mod netkeiba_scraper;
pub mod odds_scraper;
pub mod payout_fetcher;
pub mod pdf_fetcher;
pub mod pdf_parser;
pub mod repository;
pub mod result_page_fetcher;

pub use dto::horse_history::fetch::FetchHorseHistoryResponse;
pub use entry_parser::EntryParser;
pub use error::{Error, Result};
pub use interactor::Interactor;
pub use interactor::card::CardInteractor;
pub use interactor::entry::EntryInteractor;
pub use interactor::horse_history::HorseHistoryInteractor;
pub use interactor::live::{LiveFlip, LiveRaceView, LiveSummary, LiveView};
pub use interactor::odds::OddsInteractor;
pub use interactor::pdf::PdfInteractor;
pub use interactor::race::board::{
    BoardHorse, Confusion, HandicapNote, RaceBoard, recorded_axis_of,
};
pub use interactor::race::predict::{PredictionViews, RecentRunsCoverage, compose_portfolio};
pub use interactor::results::{RefreshReport, ResultsInteractor};
pub use interactor::settle::{SettleInteractor, SettleReport};
pub use netkeiba_race_id::{
    build_race_ids, netkeiba_race_id_from_paddock, paddock_race_id_from_netkeiba,
};
pub use netkeiba_scraper::{
    FetchedCard, FetchedEntry, FetchedWinOdds, HorsePastRun, NetkeibaScraper, RunnerRef,
};
pub use odds_scraper::{OddsScraper, ScrapedOdds};
pub use paddock_domain::{HorseFactors, HorseProbability, RateTriple};
pub use payout_fetcher::PayoutFetcher;
pub use pdf_fetcher::{FetchProbe, PdfFetcher};
pub use pdf_parser::PdfParser;
// `ConditionRun` / `DISTANCE_EXPERIENCE_TOLERANCE_M` は盤の出力型（`HandicapNote`）が内包する
// 過去走 1 走と、その距離判定の許容幅（#628）。定義は repository ポート側だが、`RaceBoard` を
// 消費する層（rest-controller）が `repository::` を辿らずに済むよう root から再輸出する。
pub use repository::{
    ConditionRun, CourseStatsRow, DISTANCE_EXPERIENCE_TOLERANCE_M, FetchDownload, FetchFailure,
    FetchRecord, FetchStatus, FinishEntry, GroupStat, HorseStatsRow, JockeyStatsRow, MarkStatRow,
    MarkStatsFilter, OddsRow, PredictBetRecord, PredictRaceConditionRecord, PredictSessionRecord,
    PredictionFilter, PredictionSearchResult, PredictionSummaryRow, RaceOddsRecord,
    RaceResultRepository, Repository, UnpricedObservation,
};
pub use result_page_fetcher::ResultPageFetcher;

/// `--trend-n` に指定できる最大値（= TREND_WEIGHTS の要素数）。
/// backtest CLI バリデーションと TREND_WEIGHTS の要素数を一致させるために参照する。
pub const TREND_N_MAX: u32 = interactor::race::predict::TREND_WEIGHTS.len() as u32;
