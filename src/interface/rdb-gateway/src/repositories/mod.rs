mod backfill_horse_ids;
mod course_stats;
mod fetch_history;
mod find_finished_races_between;
mod find_jockey_recent_runs;
mod find_live_ev_by_date;
mod find_matching_names;
mod find_race_card;
mod find_race_odds;
mod find_races_by_date;
mod find_recent_runs;
mod horse_history;
mod horse_stats;
mod jockey_stats;
mod pad_prediction;
mod predict_session;
mod purge_race_odds_snapshots;
mod save_race;
mod save_race_card;
mod save_race_odds;
mod sql;
mod standard_times;
mod trainer_stats;
mod update_results;

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::{
    HorseId, HorseName, JockeyFormRun, JockeyName, PadPrediction, Race, RaceCard, RaceId, RaceOdds,
    RecentRun, StandardTimes, Surface, TrainerName, Venue,
};
use paddock_use_case::Result as UcResult;
use paddock_use_case::repository::{
    CourseStatsRow, FetchDownload, FetchFailure, FetchRecord, FetchRepository, FetchStatus,
    HorseHistoryRepository, HorseRecencyStats, HorseStatsRow, JockeyStatsRow, LiveEvRepository,
    LiveEvSnapshot, MarkStatRow, MarkStatsFilter, NameMatchRepository, OddsRepository,
    PadPredictionRepository, PredictBetRecord, PredictRaceConditionRecord, PredictSessionRecord,
    PredictSessionRepository, PredictionFilter, PredictionSearchResult, RaceCardRepository,
    RaceOddsRecord, RaceRepository, StatsRepository, TrainerStatsRow,
};

use crate::pool::PgPool;

pub struct PostgresRepository {
    pub pool: PgPool,
}

impl PostgresRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// netkeiba 結果由来の clean な成績で既存 `results` 行を更新する（`fetch-results` 用）。
    /// `Repository` トレイトには載せず、結果再取込フロー専用の inherent メソッドとする。
    pub async fn update_results(
        &self,
        race_id: &RaceId,
        rows: &[paddock_use_case::netkeiba_scraper::ResultRow],
    ) -> UcResult<u64> {
        update_results::update_results(&self.pool, race_id, rows)
            .await
            .map_err(Into::into)
    }
}

impl StatsRepository for PostgresRepository {
    async fn horse_stats(
        &self,
        name: &HorseName,
        as_of: Option<NaiveDate>,
    ) -> UcResult<HorseStatsRow> {
        horse_stats::horse_stats(&self.pool, name, as_of)
            .await
            .map_err(Into::into)
    }

    async fn horse_stats_batch(
        &self,
        names: &[HorseName],
        as_of: Option<NaiveDate>,
    ) -> UcResult<HashMap<HorseName, HorseStatsRow>> {
        horse_stats::horse_stats_batch(&self.pool, names, as_of)
            .await
            .map_err(Into::into)
    }

    async fn horse_recency(
        &self,
        name: &HorseName,
        as_of: Option<NaiveDate>,
    ) -> UcResult<HorseRecencyStats> {
        horse_stats::horse_recency(&self.pool, name, as_of)
            .await
            .map_err(Into::into)
    }

    async fn horse_recency_batch(
        &self,
        names: &[HorseName],
        as_of: Option<NaiveDate>,
    ) -> UcResult<HashMap<HorseName, HorseRecencyStats>> {
        horse_stats::horse_recency_batch(&self.pool, names, as_of)
            .await
            .map_err(Into::into)
    }

    async fn course_stats(
        &self,
        venue: Venue,
        distance: u32,
        surface: Surface,
        as_of: Option<NaiveDate>,
    ) -> UcResult<CourseStatsRow> {
        course_stats::course_stats(&self.pool, venue, distance, surface, as_of)
            .await
            .map_err(Into::into)
    }

    async fn jockey_stats(
        &self,
        name: &JockeyName,
        as_of: Option<NaiveDate>,
    ) -> UcResult<JockeyStatsRow> {
        jockey_stats::jockey_stats(&self.pool, name, as_of)
            .await
            .map_err(Into::into)
    }

    async fn jockey_stats_batch(
        &self,
        names: &[JockeyName],
        as_of: Option<NaiveDate>,
    ) -> UcResult<HashMap<JockeyName, JockeyStatsRow>> {
        jockey_stats::jockey_stats_batch(&self.pool, names, as_of)
            .await
            .map_err(Into::into)
    }

    async fn trainer_stats(
        &self,
        name: &TrainerName,
        as_of: Option<NaiveDate>,
    ) -> UcResult<TrainerStatsRow> {
        trainer_stats::trainer_stats(&self.pool, name, as_of)
            .await
            .map_err(Into::into)
    }

    async fn trainer_stats_batch(
        &self,
        names: &[TrainerName],
        as_of: Option<NaiveDate>,
    ) -> UcResult<HashMap<TrainerName, TrainerStatsRow>> {
        trainer_stats::trainer_stats_batch(&self.pool, names, as_of)
            .await
            .map_err(Into::into)
    }

    async fn find_finished_races_between(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> UcResult<Vec<Race>> {
        find_finished_races_between::find_finished_races_between(&self.pool, from, to)
            .await
            .map_err(Into::into)
    }

    async fn find_recent_runs(
        &self,
        name: &HorseName,
        before: NaiveDate,
        limit: u32,
    ) -> UcResult<Vec<RecentRun>> {
        find_recent_runs::find_recent_runs(&self.pool, name, before, limit)
            .await
            .map_err(Into::into)
    }

    async fn recent_runs_batch(
        &self,
        names: &[HorseName],
        before: NaiveDate,
        limit: u32,
    ) -> UcResult<HashMap<HorseName, Vec<RecentRun>>> {
        find_recent_runs::recent_runs_batch(&self.pool, names, before, limit)
            .await
            .map_err(Into::into)
    }

    async fn find_jockey_recent_runs(
        &self,
        jockey: &JockeyName,
        before: NaiveDate,
        limit: u32,
    ) -> UcResult<Vec<JockeyFormRun>> {
        find_jockey_recent_runs::find_jockey_recent_runs(&self.pool, jockey, before, limit)
            .await
            .map_err(Into::into)
    }

    async fn jockey_recent_runs_batch(
        &self,
        jockeys: &[JockeyName],
        before: NaiveDate,
        limit: u32,
    ) -> UcResult<HashMap<JockeyName, Vec<JockeyFormRun>>> {
        find_jockey_recent_runs::jockey_recent_runs_batch(&self.pool, jockeys, before, limit)
            .await
            .map_err(Into::into)
    }

    async fn standard_times(&self, before: NaiveDate) -> UcResult<StandardTimes> {
        standard_times::standard_times(&self.pool, before)
            .await
            .map_err(Into::into)
    }
}

impl NameMatchRepository for PostgresRepository {
    async fn find_matching_horse_names(&self, query: &str, limit: u32) -> UcResult<Vec<String>> {
        find_matching_names::find_matching_horse_names(&self.pool, query, limit)
            .await
            .map_err(Into::into)
    }

    async fn find_matching_jockey_names(&self, query: &str, limit: u32) -> UcResult<Vec<String>> {
        find_matching_names::find_matching_jockey_names(&self.pool, query, limit)
            .await
            .map_err(Into::into)
    }

    async fn find_matching_trainer_names(&self, query: &str, limit: u32) -> UcResult<Vec<String>> {
        find_matching_names::find_matching_trainer_names(&self.pool, query, limit)
            .await
            .map_err(Into::into)
    }
}

impl RaceRepository for PostgresRepository {
    async fn save_race(&self, race: &Race) -> UcResult<()> {
        save_race::save_race(&self.pool, race)
            .await
            .map_err(Into::into)
    }

    async fn count_races(&self) -> UcResult<u64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM races")
            .fetch_one(&self.pool)
            .await
            .map_err(crate::Error::from)?;
        Ok(row.0 as u64)
    }

    async fn race_exists(&self, race_id: &RaceId) -> UcResult<bool> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM races WHERE race_id = $1 LIMIT 1")
            .bind(race_id.value())
            .fetch_optional(&self.pool)
            .await
            .map_err(crate::Error::from)?;
        Ok(row.is_some())
    }

    async fn find_races_by_date(&self, date: NaiveDate) -> UcResult<Vec<Race>> {
        find_races_by_date::find_races_by_date(&self.pool, date)
            .await
            .map_err(Into::into)
    }
}

impl RaceCardRepository for PostgresRepository {
    async fn save_race_card(&self, card: &RaceCard) -> UcResult<()> {
        save_race_card::save_race_card(&self.pool, card)
            .await
            .map_err(Into::into)
    }

    async fn find_race_card(&self, race_id: &RaceId) -> UcResult<Option<RaceCard>> {
        find_race_card::find_race_card(&self.pool, race_id)
            .await
            .map_err(Into::into)
    }
}

impl OddsRepository for PostgresRepository {
    async fn save_race_odds(&self, record: &RaceOddsRecord) -> UcResult<()> {
        save_race_odds::save_race_odds(&self.pool, record)
            .await
            .map_err(Into::into)
    }

    async fn find_race_odds(
        &self,
        race_id: &RaceId,
        as_of: Option<NaiveDate>,
    ) -> UcResult<Option<RaceOdds>> {
        find_race_odds::find_race_odds(&self.pool, race_id, as_of)
            .await
            .map_err(Into::into)
    }

    async fn purge_race_odds_snapshots(&self, before: NaiveDate) -> UcResult<u64> {
        purge_race_odds_snapshots::purge_race_odds_snapshots(&self.pool, before)
            .await
            .map_err(Into::into)
    }

    async fn count_race_odds_snapshots_before(&self, before: NaiveDate) -> UcResult<u64> {
        purge_race_odds_snapshots::count_race_odds_snapshots_before(&self.pool, before)
            .await
            .map_err(Into::into)
    }
}

impl FetchRepository for PostgresRepository {
    async fn fetch_history_contains(&self, source_key: &str) -> UcResult<bool> {
        fetch_history::contains(&self.pool, source_key)
            .await
            .map_err(Into::into)
    }

    async fn record_fetch(&self, record: &FetchRecord) -> UcResult<()> {
        fetch_history::record(&self.pool, record)
            .await
            .map_err(Into::into)
    }

    async fn fetch_status(&self, source_key: &str) -> UcResult<Option<FetchStatus>> {
        fetch_history::status(&self.pool, source_key)
            .await
            .map_err(Into::into)
    }

    async fn record_download(&self, record: &FetchDownload) -> UcResult<()> {
        fetch_history::record_download(&self.pool, record)
            .await
            .map_err(Into::into)
    }

    async fn record_failure(&self, record: &FetchFailure) -> UcResult<()> {
        fetch_history::record_failure(&self.pool, record)
            .await
            .map_err(Into::into)
    }
}

impl HorseHistoryRepository for PostgresRepository {
    async fn upsert_horse_history(
        &self,
        horse_id: &HorseId,
        runs: &[paddock_use_case::HorsePastRun],
    ) -> UcResult<usize> {
        horse_history::upsert_horse_history(&self.pool, horse_id, runs)
            .await
            .map_err(Into::into)
    }

    async fn backfill_results_horse_ids(&self) -> UcResult<u64> {
        backfill_horse_ids::backfill_results_horse_ids(&self.pool)
            .await
            .map_err(Into::into)
    }
}

impl PredictSessionRepository for PostgresRepository {
    async fn find_predict_session(
        &self,
        date: NaiveDate,
    ) -> UcResult<Option<PredictSessionRecord>> {
        predict_session::find_predict_session(&self.pool, date)
            .await
            .map_err(Into::into)
    }

    async fn find_predict_bets(&self, date: NaiveDate) -> UcResult<Vec<PredictBetRecord>> {
        predict_session::find_predict_bets(&self.pool, date)
            .await
            .map_err(Into::into)
    }

    async fn find_predict_bets_with_id(
        &self,
        date: NaiveDate,
    ) -> UcResult<Vec<(i64, PredictBetRecord)>> {
        predict_session::find_predict_bets_with_id(&self.pool, date)
            .await
            .map_err(Into::into)
    }

    async fn settle_predict_session(
        &self,
        session: &PredictSessionRecord,
        settled: &[(i64, u64)],
    ) -> UcResult<()> {
        predict_session::settle_predict_session(&self.pool, session, settled)
            .await
            .map_err(Into::into)
    }

    async fn save_predict_session(&self, session: &PredictSessionRecord) -> UcResult<()> {
        predict_session::save_predict_session(&self.pool, session)
            .await
            .map_err(Into::into)
    }

    async fn save_race_outcome(
        &self,
        session: &PredictSessionRecord,
        race_id: &RaceId,
        bets: &[PredictBetRecord],
    ) -> UcResult<()> {
        predict_session::save_race_outcome(&self.pool, session, race_id, bets)
            .await
            .map_err(Into::into)
    }

    async fn find_predict_race_conditions(
        &self,
        date: NaiveDate,
    ) -> UcResult<Vec<PredictRaceConditionRecord>> {
        predict_session::find_predict_race_conditions(&self.pool, date)
            .await
            .map_err(Into::into)
    }

    async fn save_predict_race_condition(
        &self,
        date: NaiveDate,
        record: &PredictRaceConditionRecord,
        recorded_at: DateTime<Utc>,
    ) -> UcResult<()> {
        predict_session::save_predict_race_condition(&self.pool, date, record, recorded_at)
            .await
            .map_err(Into::into)
    }
}

impl LiveEvRepository for PostgresRepository {
    async fn find_live_ev_by_date(&self, date: NaiveDate) -> UcResult<Vec<LiveEvSnapshot>> {
        find_live_ev_by_date::find_live_ev_by_date(&self.pool, date)
            .await
            .map_err(Into::into)
    }
}

impl PadPredictionRepository for PostgresRepository {
    async fn save_pad_prediction(
        &self,
        prediction: &PadPrediction,
        now: DateTime<Utc>,
    ) -> UcResult<()> {
        pad_prediction::save_pad_prediction(&self.pool, prediction, now)
            .await
            .map_err(Into::into)
    }

    async fn find_pad_prediction(
        &self,
        date: NaiveDate,
        venue: Venue,
        race_num: u32,
    ) -> UcResult<Option<PadPrediction>> {
        pad_prediction::find_pad_prediction(&self.pool, date, venue, race_num)
            .await
            .map_err(Into::into)
    }

    async fn list_pad_predictions(&self) -> UcResult<Vec<PadPrediction>> {
        pad_prediction::list_pad_predictions(&self.pool)
            .await
            .map_err(Into::into)
    }

    async fn search_predictions(
        &self,
        filter: &PredictionFilter,
    ) -> UcResult<PredictionSearchResult> {
        pad_prediction::search_predictions(&self.pool, filter)
            .await
            .map_err(Into::into)
    }

    async fn find_pad_prediction_by_id(
        &self,
        prediction_id: i64,
    ) -> UcResult<Option<PadPrediction>> {
        pad_prediction::find_pad_prediction_by_id(&self.pool, prediction_id)
            .await
            .map_err(Into::into)
    }

    async fn prediction_mark_stats(&self, filter: &MarkStatsFilter) -> UcResult<Vec<MarkStatRow>> {
        pad_prediction::prediction_mark_stats(&self.pool, filter)
            .await
            .map_err(Into::into)
    }
}
