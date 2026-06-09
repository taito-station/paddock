use core::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::{
    HorseId, HorseName, HorseResult, JockeyName, Race, RaceCard, RaceId, Surface, Venue,
};

use crate::error::Result;
use crate::netkeiba_scraper::HorsePastRun;

#[derive(Debug, Clone)]
pub struct GroupStat {
    pub label: String,
    pub starts: u32,
    pub wins: u32,
    pub places: u32,
    pub shows: u32,
}

impl GroupStat {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            starts: 0,
            wins: 0,
            places: 0,
            shows: 0,
        }
    }

    pub fn win_rate(&self) -> f64 {
        if self.starts == 0 {
            0.0
        } else {
            self.wins as f64 / self.starts as f64
        }
    }

    pub fn place_rate(&self) -> f64 {
        if self.starts == 0 {
            0.0
        } else {
            self.places as f64 / self.starts as f64
        }
    }

    pub fn show_rate(&self) -> f64 {
        if self.starts == 0 {
            0.0
        } else {
            self.shows as f64 / self.starts as f64
        }
    }
}

#[derive(Debug, Clone)]
pub struct HorseStatsRow {
    pub horse_name: String,
    pub by_surface: Vec<GroupStat>,
    pub by_distance_band: Vec<GroupStat>,
    pub by_gate_group: Vec<GroupStat>,
    pub by_track_condition: Vec<GroupStat>,
    pub by_popularity_band: Vec<GroupStat>,
    pub overall: GroupStat,
}

#[derive(Debug, Clone)]
pub struct CourseStatsRow {
    pub venue: String,
    pub distance: u32,
    pub surface: String,
    pub by_gate_group: Vec<GroupStat>,
}

#[derive(Debug, Clone)]
pub struct JockeyStatsRow {
    pub jockey_name: String,
    pub overall: GroupStat,
    pub by_surface: Vec<GroupStat>,
    pub by_gate_group: Vec<GroupStat>,
}

/// A successful fetch+ingest of a JRA meeting-day PDF, persisted so the same
/// meeting is not re-fetched on a later run (exclusive control).
#[derive(Debug, Clone)]
pub struct FetchRecord {
    pub source_key: String,
    pub url: String,
    pub races_saved: u32,
    pub horses_saved: u32,
    /// When the fetch+ingest happened. Set by the use-case layer so the gateway
    /// stays free of clock side effects (and tests can control it).
    pub fetched_at: DateTime<Utc>,
}

/// 予想セッション 1 件（1 開催日 = 1 セッション）。途中離脱後の `--resume` と
/// 収支サマリ `--summary` のために永続化する。`created_at`/`updated_at` は use-case 層が
/// 時刻を注入し、gateway を時計から独立に保つ（[`FetchRecord`] と同じ流儀）。
#[derive(Debug, Clone)]
pub struct PredictSessionRecord {
    pub date: NaiveDate,
    pub budget: u64,
    pub balance: u64,
    pub total_bet: u64,
    pub total_payout: u64,
    pub completed: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `race_odds` テーブルへ保存する 1 行分のオッズ。ドメインの [`RaceOdds`] は
/// popularity / fetched_at を持てないため、永続化専用の入力型として use-case 層に定義する。
#[derive(Debug, Clone)]
pub struct OddsRow {
    /// 馬券種ラベル（単勝なら `"win"`）。
    pub bet_type: String,
    /// 組み合わせキー（単勝なら馬番文字列）。
    pub combination_key: String,
    pub odds: f64,
    /// 複勝のような下限/上限を持つ馬券種の上限値。単勝は `None`。
    pub odds_high: Option<f64>,
    pub popularity: Option<u32>,
}

/// 1 レース分のオッズ取得結果。取得時刻 `fetched_at` は use-case 層で注入し、
/// gateway を時計から独立に保つ（[`FetchRecord`] と同じ流儀）。
#[derive(Debug, Clone)]
pub struct RaceOddsRecord {
    pub race_id: RaceId,
    pub fetched_at: DateTime<Utc>,
    pub rows: Vec<OddsRow>,
}

/// セッション内で実際に購入した買い目 1 件。払戻は買い目ごと（per-bet）に記録する。
#[derive(Debug, Clone)]
pub struct PredictBetRecord {
    pub race_id: RaceId,
    /// 馬券種ラベル（`BetCombination::type_label`）。
    pub bet_type: String,
    /// 組み合わせコード（`BetCombination::combination_code`）。
    pub combination: String,
    pub stake: u64,
    pub payout: u64,
    pub ev: f64,
}

pub trait Repository: Send + Sync {
    fn save_race(&self, race: &Race) -> impl Future<Output = Result<()>> + Send;

    /// netkeiba 由来の近走を horse 単位で `horses` / `horse_past_runs` に upsert する。
    /// pdf 確定成績(`results`)とは別テーブルに保存することで、集計の二重計上・フィールド
    /// バイアス（#58/#59）を構造的に防ぐ。`runs` が空のときは何もしない。
    fn upsert_horse_history(
        &self,
        horse_id: &HorseId,
        runs: &[HorsePastRun],
    ) -> impl Future<Output = Result<()>> + Send;

    /// `horses` マスタに馬名がちょうど 1 件一致する pdf 成績行（`results.horse_id IS NULL`）へ
    /// horse_id を backfill する。同名別馬（複数一致）・不一致は NULL のまま残し、既存値は
    /// 上書きしない（冪等）。埋めた行数を返す。
    fn backfill_results_horse_ids(&self) -> impl Future<Output = Result<u64>> + Send;

    /// 馬の各種成績統計を返す。`as_of = Some(d)` のとき `races.date < d` の成績のみを集計する
    /// （バックテストのリーク防止。本番予想は `None` で全期間集計）。
    fn horse_stats(
        &self,
        name: &HorseName,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HorseStatsRow>> + Send;

    /// コース（場×距離×馬場）の枠順別統計を返す。`as_of` の意味は [`Repository::horse_stats`] と同じ。
    fn course_stats(
        &self,
        venue: Venue,
        distance: u32,
        surface: Surface,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<CourseStatsRow>> + Send;

    /// 騎手の各種成績統計を返す。`as_of` の意味は [`Repository::horse_stats`] と同じ。
    fn jockey_stats(
        &self,
        name: &JockeyName,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<JockeyStatsRow>> + Send;

    /// 指定期間 `[from, to]`（両端含む）の確定済みレースを `results` 付きで race_num 昇順に返す。
    /// `races.source='pdf'` かつ着順ありの `results` を 1 件以上含むレースのみを対象とする
    /// （バックテストの評価対象取得用）。`from > to` のときは空 Vec を返す。
    fn find_finished_races_between(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> impl Future<Output = Result<Vec<Race>>> + Send;

    /// 指定馬の `before` より前（`races.date < before`）の成績を date 降順で最大 `limit` 件返す。
    /// 各要素は `(開催日, 成績)`。前走フォーム特徴量（#31）の算出に使う。`before` 制約により
    /// バックステスト時のリークを防ぐ。pdf/netkeiba 双方の成績を対象とする（実際の前走）。
    fn find_recent_runs(
        &self,
        name: &HorseName,
        before: NaiveDate,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<(NaiveDate, HorseResult)>>> + Send;

    fn count_races(&self) -> impl Future<Output = Result<u64>> + Send;

    fn race_exists(&self, race_id: &RaceId) -> impl Future<Output = Result<bool>> + Send;

    /// Whether a meeting-day source key has already been ingested.
    fn fetch_history_contains(&self, source_key: &str)
    -> impl Future<Output = Result<bool>> + Send;

    /// Record a successful meeting-day fetch+ingest in the history table.
    fn record_fetch(&self, record: &FetchRecord) -> impl Future<Output = Result<()>> + Send;

    fn save_race_card(&self, card: &RaceCard) -> impl Future<Output = Result<()>> + Send;

    /// 1 レース分のオッズ（行単位）を upsert する。`race_odds` の主キー
    /// `(race_id, bet_type, combination_key)` で衝突した行は最新値で更新する。
    fn save_race_odds(&self, record: &RaceOddsRecord) -> impl Future<Output = Result<()>> + Send;

    fn find_race_card(
        &self,
        race_id: &RaceId,
    ) -> impl Future<Output = Result<Option<RaceCard>>> + Send;

    /// 指定日に開催されるレース一覧を race_num 昇順で返す。
    /// 予想用途のため `results` は読み込まず空 Vec で返す。
    fn find_races_by_date(&self, date: NaiveDate)
    -> impl Future<Output = Result<Vec<Race>>> + Send;

    /// 指定日の予想セッションを返す。未作成なら `None`。
    fn find_predict_session(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<Option<PredictSessionRecord>>> + Send;

    /// 指定日のセッションで購入済みの買い目を bet_id 昇順で返す。
    /// `--summary` の明細表示と `--resume` の処理済みレース判定に使う。
    fn find_predict_bets(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<Vec<PredictBetRecord>>> + Send;

    /// 予想セッションのヘッダ（残高・累計・completed）を upsert する。
    /// 新規開始時の作成と、全レース処理後の完了マークに使う。
    fn save_predict_session(
        &self,
        session: &PredictSessionRecord,
    ) -> impl Future<Output = Result<()>> + Send;

    /// 1 レース分の確定結果を 1 トランザクションで保存する。
    /// セッション行を upsert（残高・累計・completed・updated_at を更新）し、
    /// その race の買い目 `bets` を追記する。
    fn save_race_outcome(
        &self,
        session: &PredictSessionRecord,
        race_id: &RaceId,
        bets: &[PredictBetRecord],
    ) -> impl Future<Output = Result<()>> + Send;
}
