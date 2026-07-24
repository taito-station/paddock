//! Repository トレイト群（ISP 分割済み）と DTO を、責務ごとのサブモジュールに分けて定義する。
//! 従来 `repository.rs` 単一ファイルに同居していた DTO・全 sub-trait・合成 `Repository` を、
//! import パスを変えずに機械的に分割したもの（#454）。各サブモジュールの公開型は glob で
//! re-export し、`crate::repository::X` / `paddock_use_case::repository::X` の従来パスを維持する。

pub mod fetch;
pub mod live;
pub mod odds;
pub mod race;
pub mod session;
pub mod stats;

pub use fetch::HorseHistoryRepository;
pub use fetch::{FetchDownload, FetchFailure, FetchRecord, FetchRepository, FetchStatus};
pub use live::{LiveEvRepository, LiveEvSnapshot, LiveEvSnapshotRecord, SlipLegRecord};
pub use odds::{MorningRaceOdds, OddsRepository, OddsRow, RaceOddsRecord};
pub use race::{FinishEntry, RaceCardRepository, RaceRepository, RaceResultRepository};
pub use session::{
    PadPredictionRepository, PredictBetRecord, PredictRaceConditionRecord, PredictSessionRecord,
    PredictSessionRepository, PredictionFilter, PredictionSearchResult, PredictionSummaryRow,
};
pub use stats::{
    ConditionalGateStatsRow, CourseStatsRow, GATE_FIELD_BANDS, GATE_TRACK_FIRM, GATE_TRACK_OTHER,
    GateBiasCell, GroupStat, HorseRecencyStats, HorseStatsRow, JockeyStatsRow, MarkStatRow,
    MarkStatsFilter, NameMatchRepository, RecencySeries, StatsRepository, TrainerStatsRow,
    gate_field_band_label, gate_track_cond2_label,
};

/// 後方互換のための集約スーパートレイト。全 sub-trait を満たす型に blanket 実装される。
/// `Send + Sync` は各 sub-trait が既に要求するため、ここでは再列挙しない。
pub trait Repository:
    StatsRepository
    + NameMatchRepository
    + RaceRepository
    + RaceCardRepository
    + OddsRepository
    + FetchRepository
    + HorseHistoryRepository
    + PredictSessionRepository
    + PadPredictionRepository
    + LiveEvRepository
    + RaceResultRepository
{
}

impl<T> Repository for T where
    T: StatsRepository
        + NameMatchRepository
        + RaceRepository
        + RaceCardRepository
        + OddsRepository
        + FetchRepository
        + HorseHistoryRepository
        + PredictSessionRepository
        + PadPredictionRepository
        + LiveEvRepository
        + RaceResultRepository
{
}
