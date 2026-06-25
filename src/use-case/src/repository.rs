use core::future::Future;
use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::{
    BetType, HorseId, HorseName, JockeyFormRun, JockeyName, Mark, OrderedPair, OrderedTriple,
    PadPrediction, Pair, Race, RaceCard, RaceId, RaceOdds, RecentRun, StandardTimes, Surface,
    TrackCondition, TrainerName, Triple, Venue,
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

/// recency 重み付け（#75 Phase B）用に、あるカテゴリの 1 ラベルぶんの日付付き成績系列。
/// `runs` は `races.date < as_of` の各開催日のカウント（同一日複数走は 1 要素にまとめる）。
#[derive(Debug, Clone, Default)]
pub struct RecencySeries {
    pub label: String,
    pub runs: Vec<paddock_domain::DatedCounts>,
}

/// 馬の成績を recency 重み付け用に「カテゴリ × ラベル別の日付付き系列」で返す（#75 Phase B）。
/// 集計済み [`HorseStatsRow`] と違い各開催日のカウントを保持し、domain 側で時間減衰を掛ける。
/// recency 無効時は取得しない（mock・既定実装は空）。
#[derive(Debug, Clone, Default)]
pub struct HorseRecencyStats {
    pub by_surface: Vec<RecencySeries>,
    pub by_distance_band: Vec<RecencySeries>,
    pub by_track_condition: Vec<RecencySeries>,
}

#[derive(Debug, Clone)]
pub struct JockeyStatsRow {
    pub jockey_name: String,
    pub overall: GroupStat,
    pub by_surface: Vec<GroupStat>,
    pub by_gate_group: Vec<GroupStat>,
}

#[derive(Debug, Clone)]
pub struct TrainerStatsRow {
    pub trainer_name: String,
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

/// 取得ライフサイクルの状態（#147 fetch/parse ステージ分割）。
/// `fetch_history` の 1 開催日がどこまで進んだかを表す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchStatus {
    /// PDF を inbox に保存済みだが未 ingest（Stage1 完了）。
    Downloaded,
    /// parse+保存まで完了（Stage2 完了）。
    Ingested,
    /// 取得が 403/404 で失敗した境界開催日（#170 / ADR0024 論点1）。除外フラグではなく
    /// 「再試行の入力」。`Downloaded`/`Ingested` と違い dedup の skip 対象にしない。
    Failed,
}

impl FetchStatus {
    /// DB 文字列から復元する。未知の値は `None`。
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "downloaded" => Some(FetchStatus::Downloaded),
            "ingested" => Some(FetchStatus::Ingested),
            "failed" => Some(FetchStatus::Failed),
            _ => None,
        }
    }
}

/// Stage1（ダウンロードのみ）の記録。PDF を inbox に保存しただけで未 ingest。
/// 時刻は use-case 層が注入する（[`FetchRecord`] と同じ流儀）。
#[derive(Debug, Clone)]
pub struct FetchDownload {
    pub source_key: String,
    pub url: String,
    pub downloaded_at: DateTime<Utc>,
}

/// 取得失敗（403/404）の記録（#170 / ADR0024 論点1）。再試行の入力として残す。
/// 時刻は use-case 層が注入する（[`FetchRecord`] と同じ流儀）。
#[derive(Debug, Clone)]
pub struct FetchFailure {
    pub source_key: String,
    pub url: String,
    /// 不在を返した HTTP ステータス（403 or 404）。
    pub http_status: u16,
    pub attempted_at: DateTime<Utc>,
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

impl OddsRow {
    /// 単勝 1 行。`combination_key` は素の馬番文字列（"1".."18"、ゼロ詰めしない）。
    pub fn win(horse_num: u32, odds: f64, popularity: Option<u32>) -> Self {
        Self {
            bet_type: BetType::Win.to_string(),
            combination_key: horse_num.to_string(),
            odds,
            odds_high: None,
            popularity,
        }
    }

    /// 複勝 1 行。幅 odds を `odds`=下限・`odds_high`=上限 に詰める。
    /// `combination_key` は素の馬番文字列（単勝と同じ規約）。
    pub fn place(horse_num: u32, low: f64, high: f64, popularity: Option<u32>) -> Self {
        Self {
            bet_type: BetType::Place.to_string(),
            combination_key: horse_num.to_string(),
            odds: low,
            odds_high: Some(high),
            popularity,
        }
    }

    // 組合せ券種(#38)はライブスクレイプ由来で人気を持たないため popularity は None 固定。

    /// 馬連 1 行。キーは昇順 `Pair`（`"1-2"`）。
    pub fn quinella(pair: Pair, odds: f64) -> Self {
        Self {
            bet_type: BetType::Quinella.to_string(),
            combination_key: pair.to_key(),
            odds,
            odds_high: None,
            popularity: None,
        }
    }

    /// ワイド 1 行。複勝と同じく幅 odds（`odds`=下限・`odds_high`=上限）。キーは昇順 `Pair`。
    pub fn wide(pair: Pair, low: f64, high: f64) -> Self {
        Self {
            bet_type: BetType::Wide.to_string(),
            combination_key: pair.to_key(),
            odds: low,
            odds_high: Some(high),
            popularity: None,
        }
    }

    /// 馬単 1 行。キーは順序付き `OrderedPair`（`"1>2"`）。
    pub fn exacta(pair: OrderedPair, odds: f64) -> Self {
        Self {
            bet_type: BetType::Exacta.to_string(),
            combination_key: pair.to_key(),
            odds,
            odds_high: None,
            popularity: None,
        }
    }

    /// 三連複 1 行。キーは昇順 `Triple`（`"1-2-3"`）。
    pub fn trio(triple: Triple, odds: f64) -> Self {
        Self {
            bet_type: BetType::Trio.to_string(),
            combination_key: triple.to_key(),
            odds,
            odds_high: None,
            popularity: None,
        }
    }

    /// 三連単 1 行。キーは順序付き `OrderedTriple`（`"1>2>3"`）。
    pub fn trifecta(triple: OrderedTriple, odds: f64) -> Self {
        Self {
            bet_type: BetType::Trifecta.to_string(),
            combination_key: triple.to_key(),
            odds,
            odds_high: None,
            popularity: None,
        }
    }
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

/// 予想セッションで 1 レースの冒頭に対話入力した馬場状態（#80）。レース単位で記録し、
/// 「どの馬場前提で予想したか」を再現・監査できるようにする。`track_condition` が `None` は
/// 「不明として入力した」ことを表す（レコードの存在自体が「入力済み」を意味する）。
#[derive(Debug, Clone)]
pub struct PredictRaceConditionRecord {
    pub race_id: RaceId,
    pub track_condition: Option<TrackCondition>,
}

/// 予想横断検索（#145）のフィルタ。指定された軸のみ AND で絞り、未指定（None）は素通し。
/// `limit` / `offset` はページング。`horse_name` は正規化済み・未エスケープの素の文字列を渡し、
/// gateway 側で LIKE のワイルドカードをリテラル化する。
#[derive(Debug, Clone)]
pub struct PredictionFilter {
    pub date_from: Option<NaiveDate>,
    pub date_to: Option<NaiveDate>,
    pub venue: Option<Venue>,
    pub distance_min: Option<u32>,
    pub distance_max: Option<u32>,
    pub surface: Option<Surface>,
    /// 馬名の部分一致（カナ正規化済み）。指定時、その馬を含む予想のみに絞る。
    pub horse_name: Option<String>,
    /// 印。指定時、その印を付けた馬を含む予想のみ。`horse_name` と併用すると同一馬が両条件を満たす。
    pub mark: Option<Mark>,
    /// 的中フィルタ。`Some(true)`=的中（`recovery_rate > 0`）、`Some(false)`=不的中
    /// （結果あり且つ払戻 0 以下）、`None`=結果有無を問わず全件。
    pub hit: Option<bool>,
    pub limit: u32,
    pub offset: u32,
}

/// 検索一覧の 1 行（サマリ）。馬・買い目の全量は持たず、個別取得で補う。
/// `distance` / `surface` は `races` 結合で得た値（未照合なら `None`）。
#[derive(Debug, Clone)]
pub struct PredictionSummaryRow {
    pub prediction_id: i64,
    pub date: NaiveDate,
    pub venue: Venue,
    pub race_num: u32,
    pub race_id: Option<String>,
    pub title: Option<String>,
    pub distance: Option<u32>,
    pub surface: Option<Surface>,
    /// 印 ◎（本命）の馬名（◎が複数なら horse_num 昇順の先頭。無ければ `None`）。
    pub honmei_horse: Option<String>,
    /// `[finish_1, finish_2, finish_3]`（馬番）。結果未記録なら `None`。
    pub finish: Option<[Option<u32>; 3]>,
    pub recovery_rate: Option<f64>,
    pub pnl: Option<i64>,
    /// 的中判定。`recovery_rate > 0` で `Some(true)`、結果あり（`finish_1` あり）かつ払戻 0 以下で
    /// `Some(false)`、結果未記録なら `None`（`PredictionFilter::hit` フィルタと同じ集合）。
    pub hit: Option<bool>,
}

/// 検索結果（サマリ配列 + フィルタ適用後の総件数）。`total_count` で SPA がページャを組む。
#[derive(Debug, Clone)]
pub struct PredictionSearchResult {
    pub total_count: u64,
    pub summaries: Vec<PredictionSummaryRow>,
}

/// 印別集計（#145）の母集団フィルタ。母集団は結果記録済み（`finish_1 IS NOT NULL`）の予想に限る。
#[derive(Debug, Clone, Default)]
pub struct MarkStatsFilter {
    pub date_from: Option<NaiveDate>,
    pub date_to: Option<NaiveDate>,
    pub venue: Option<Venue>,
}

/// 印 1 種の的中率集計。`count` はその印が付いた（結果記録済みの）馬の延べ数。
/// `win` = 1 着、`show` = 複勝圏（3 着内）に入った延べ数。
#[derive(Debug, Clone)]
pub struct MarkStatRow {
    pub mark: Mark,
    pub count: u32,
    pub win: u32,
    pub show: u32,
}

impl MarkStatRow {
    pub fn win_rate(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.win as f64 / self.count as f64
        }
    }

    pub fn show_rate(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.show as f64 / self.count as f64
        }
    }
}

/// 馬・騎手・調教師・コースの成績統計と、標準タイム・近走・確定レース系の読み出し。
pub trait StatsRepository: Send + Sync {
    /// 馬の各種成績統計を返す。`as_of = Some(d)` のとき `races.date < d` の成績のみを集計する
    /// （バックテストのリーク防止。本番予想は `None` で全期間集計）。
    fn horse_stats(
        &self,
        name: &HorseName,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HorseStatsRow>> + Send;

    /// 複数馬の `horse_stats` をまとめて返す（#196 backtest の N+1 解消）。`names` の各馬を
    /// キーに [`HorseStatsRow`] を引く。`as_of` の意味は [`StatsRepository::horse_stats`] と同じ。
    /// 既定実装は per-item `horse_stats` をループするだけで挙動は変わらない（mock・predict 経路は
    /// この既定で十分。rdb-gateway のみが 1 レース一括クエリで override する）。返却 map は `names`
    /// に現れた全馬のエントリを含む（重複名は 1 回だけ引く）。
    fn horse_stats_batch(
        &self,
        names: &[HorseName],
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HashMap<HorseName, HorseStatsRow>>> + Send {
        async move {
            let mut out = HashMap::new();
            for name in names {
                if out.contains_key(name) {
                    continue;
                }
                out.insert(name.clone(), self.horse_stats(name, as_of).await?);
            }
            Ok(out)
        }
    }

    /// recency 重み付け（#75 Phase B）用に、馬の成績を「カテゴリ × ラベル別の日付付き系列」で返す。
    /// `as_of` の意味は [`StatsRepository::horse_stats`] と同じ（`races.date < as_of`）。既定実装は空を返す
    /// （recency 無効時の本番経路・テスト mock はこの既定で十分。日付付き集計が要るのは
    /// rdb-gateway のみがオーバーライドする）。
    fn horse_recency(
        &self,
        _name: &HorseName,
        _as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HorseRecencyStats>> + Send {
        async { Ok(HorseRecencyStats::default()) }
    }

    /// 複数馬の `horse_recency` をまとめて返す（#196）。既定実装は per-item `horse_recency` を
    /// ループするだけで挙動は変わらない（既定 `horse_recency` は空を返すため、recency 無効時の
    /// 本番経路・mock では空 map ではなく全馬の空系列が入る点に注意）。rdb-gateway のみが
    /// 1 レース一括クエリで override する。返却 map は `names` の全馬を含む。
    /// なお、この既定実装が返す「全馬の空系列」を実際に使うのは default 実装を踏む経路（mock 等）
    /// だけで、backtest は recency 無効時にそもそも本メソッドを呼ばない（呼ぶのは Postgres override のみ）。
    fn horse_recency_batch(
        &self,
        names: &[HorseName],
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HashMap<HorseName, HorseRecencyStats>>> + Send {
        async move {
            let mut out = HashMap::new();
            for name in names {
                if out.contains_key(name) {
                    continue;
                }
                out.insert(name.clone(), self.horse_recency(name, as_of).await?);
            }
            Ok(out)
        }
    }

    /// コース（場×距離×馬場）の枠順別統計を返す。`as_of` の意味は [`StatsRepository::horse_stats`] と同じ。
    fn course_stats(
        &self,
        venue: Venue,
        distance: u32,
        surface: Surface,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<CourseStatsRow>> + Send;

    /// 騎手の各種成績統計を返す。`as_of` の意味は [`StatsRepository::horse_stats`] と同じ。
    fn jockey_stats(
        &self,
        name: &JockeyName,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<JockeyStatsRow>> + Send;

    /// 複数騎手の `jockey_stats` をまとめて返す（#196）。既定実装は per-item をループするだけ。
    /// rdb-gateway のみが 1 レース一括クエリで override する。返却 map は `names` の全騎手を含む。
    fn jockey_stats_batch(
        &self,
        names: &[JockeyName],
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HashMap<JockeyName, JockeyStatsRow>>> + Send {
        async move {
            let mut out = HashMap::new();
            for name in names {
                if out.contains_key(name) {
                    continue;
                }
                out.insert(name.clone(), self.jockey_stats(name, as_of).await?);
            }
            Ok(out)
        }
    }

    /// 調教師の各種成績統計を返す。`as_of` の意味は [`StatsRepository::horse_stats`] と同じ。
    fn trainer_stats(
        &self,
        name: &TrainerName,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<TrainerStatsRow>> + Send;

    /// 複数調教師の `trainer_stats` をまとめて返す（#196）。既定実装は per-item をループするだけ。
    /// rdb-gateway のみが 1 レース一括クエリで override する。返却 map は `names` の全調教師を含む。
    fn trainer_stats_batch(
        &self,
        names: &[TrainerName],
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HashMap<TrainerName, TrainerStatsRow>>> + Send {
        async move {
            let mut out = HashMap::new();
            for name in names {
                if out.contains_key(name) {
                    continue;
                }
                out.insert(name.clone(), self.trainer_stats(name, as_of).await?);
            }
            Ok(out)
        }
    }

    /// 指定期間 `[from, to]`（両端含む）の確定済みレースを `results` 付きで race_num 昇順に返す。
    /// `races.source='pdf'` かつ着順ありの `results` を 1 件以上含むレースのみを対象とする
    /// （バックテストの評価対象取得用）。`from > to` のときは空 Vec を返す。
    ///
    /// 命名は race 寄りだが、backtest の評価対象を集計読み出しする用途のため `StatsRepository` に置く
    /// （backtest interactor の束縛を `StatsRepository + OddsRepository` に閉じ、mock を最小化するため）。
    fn find_finished_races_between(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> impl Future<Output = Result<Vec<Race>>> + Send;

    /// 指定馬の `before` より前（`races.date < before`）の成績を date 降順で最大 `limit` 件返す。
    /// 各要素は `RecentRun`（開催日・当該レースの surface/distance・成績）。前走フォーム特徴量
    /// （#31/#76）の算出に使う。surface/distance は前走タイムを標準タイムへ突き合わせるために運ぶ。
    /// `before` 制約によりバックテスト時のリークを防ぐ。pdf/netkeiba 双方の成績を対象とする（実際の前走）。
    fn find_recent_runs(
        &self,
        name: &HorseName,
        before: NaiveDate,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<RecentRun>>> + Send;

    /// 複数馬の `find_recent_runs` をまとめて返す（#196）。各馬につき `before` より前の直近 `limit`
    /// 件を date 降順で引く。既定実装は per-item `find_recent_runs` をループするだけで挙動は変わらない
    /// （mock・predict 経路はこの既定で十分）。rdb-gateway のみが全馬一括の window 関数で override
    /// する。返却 map は `names` の全馬を含む（前走が無い馬は空 `Vec`）。
    fn recent_runs_batch(
        &self,
        names: &[HorseName],
        before: NaiveDate,
        limit: u32,
    ) -> impl Future<Output = Result<HashMap<HorseName, Vec<RecentRun>>>> + Send {
        async move {
            let mut out = HashMap::new();
            for name in names {
                if out.contains_key(name) {
                    continue;
                }
                out.insert(
                    name.clone(),
                    self.find_recent_runs(name, before, limit).await?,
                );
            }
            Ok(out)
        }
    }

    /// 指定騎手の `before` より前（`races.date < before`）の近走を date 降順で最大 `limit` 件返す
    /// （#221）。戻り要素 `JockeyFormRun` は着順・人気のみを運ぶ（フォームシグナル算出に特化）。
    /// `before` 制約によりバックテスト時のリークを防ぐ。pdf/netkeiba 双方の成績を対象とする。
    fn find_jockey_recent_runs(
        &self,
        jockey: &JockeyName,
        before: NaiveDate,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<JockeyFormRun>>> + Send;

    /// 複数騎手の `find_jockey_recent_runs` をまとめて返す（#221）。既定実装は per-item ループ。
    /// rdb-gateway のみが全騎手一括クエリで override する。返却 map は `jockeys` の全騎手を含む
    /// （近走が無い騎手は空 `Vec`）。
    fn jockey_recent_runs_batch(
        &self,
        jockeys: &[JockeyName],
        before: NaiveDate,
        limit: u32,
    ) -> impl Future<Output = Result<HashMap<JockeyName, Vec<JockeyFormRun>>>> + Send {
        async move {
            let mut out = HashMap::new();
            for j in jockeys {
                if out.contains_key(j) {
                    continue;
                }
                out.insert(
                    j.clone(),
                    self.find_jockey_recent_runs(j, before, limit).await?,
                );
            }
            Ok(out)
        }
    }

    /// `before` より前（`races.date < before`）のコーパスから (surface, distance) 別の標準タイム
    /// （代表タイム[秒]）を集計して返す（#76）。前走タイムを相対速度シグナルへ変換する分母に使う。
    /// `before` 制約で as-of リークを防ぐ。標本数が閾値未満の薄いバケツは含めない。
    fn standard_times(
        &self,
        before: NaiveDate,
    ) -> impl Future<Output = Result<StandardTimes>> + Send;
}

/// `analyze` の部分一致候補（馬名・騎手名・調教師名）の検索。
pub trait NameMatchRepository: Send + Sync {
    /// `analyze` の部分一致検索用。`results` に `query` を中間一致（`LIKE '%query%'`）する
    /// 馬名を重複排除して名前昇順で最大 `limit` 件返す。`query` は呼び出し側で正規化済みとする。
    fn find_matching_horse_names(
        &self,
        query: &str,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;

    /// 騎手名版（[`NameMatchRepository::find_matching_horse_names`] と同方針）。
    fn find_matching_jockey_names(
        &self,
        query: &str,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;

    /// 調教師名版（[`NameMatchRepository::find_matching_horse_names`] と同方針）。
    fn find_matching_trainer_names(
        &self,
        query: &str,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;
}

/// レース本体（確定成績）の保存と存在判定・件数・日付検索。
pub trait RaceRepository: Send + Sync {
    fn save_race(&self, race: &Race) -> impl Future<Output = Result<()>> + Send;

    fn count_races(&self) -> impl Future<Output = Result<u64>> + Send;

    fn race_exists(&self, race_id: &RaceId) -> impl Future<Output = Result<bool>> + Send;

    /// 指定日に開催されるレース一覧を race_num 昇順で返す。
    /// 予想用途のため `results` は読み込まず空 Vec で返す。
    fn find_races_by_date(&self, date: NaiveDate)
    -> impl Future<Output = Result<Vec<Race>>> + Send;
}

/// 出馬表（race card）の保存・取得。
pub trait RaceCardRepository: Send + Sync {
    fn save_race_card(&self, card: &RaceCard) -> impl Future<Output = Result<()>> + Send;

    fn find_race_card(
        &self,
        race_id: &RaceId,
    ) -> impl Future<Output = Result<Option<RaceCard>>> + Send;
}

/// レースオッズ（`race_odds`）の保存・取得。
pub trait OddsRepository: Send + Sync {
    /// 1 レース分のオッズ（行単位）を upsert する。`race_odds` の主キー
    /// `(race_id, bet_type, combination_key)` で衝突した行は最新値で更新する。
    fn save_race_odds(&self, record: &RaceOddsRecord) -> impl Future<Output = Result<()>> + Send;

    /// `race_odds` に保存済みのオッズを全券種読み出してドメインの [`RaceOdds`] に再構成する。
    /// `as_of = Some(d)` のとき `date(fetched_at) <= d` のスナップショットのみ対象とする
    /// （backtest の当時オッズ参照用、リーク防止）。`None` は時刻制約なし（predict の最新参照用）。
    /// いずれの券種の行も無ければ `None` を返す。
    fn find_race_odds(
        &self,
        race_id: &RaceId,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<Option<RaceOdds>>> + Send;
}

/// 取り込み履歴（fetch history）の存在判定・記録。
pub trait FetchRepository: Send + Sync {
    /// Whether a meeting-day source key has already been **ingested**
    /// (Stage2 完了)。ダウンロード済み・未 ingest（Stage1 のみ）は `false`。
    fn fetch_history_contains(&self, source_key: &str)
    -> impl Future<Output = Result<bool>> + Send;

    /// Record a successful meeting-day fetch+ingest in the history table
    /// （status を `ingested` にする。Stage2 完了の記録）。
    fn record_fetch(&self, record: &FetchRecord) -> impl Future<Output = Result<()>> + Send;

    /// 取得ライフサイクルの現在状態を返す（履歴に無ければ `None`）。
    /// Stage1 の dedup（ダウンロード済み or ingest 済みなら再取得不要）に使う。
    fn fetch_status(
        &self,
        source_key: &str,
    ) -> impl Future<Output = Result<Option<FetchStatus>>> + Send;

    /// Stage1: ダウンロード済み（inbox 保存済み・未 ingest）を記録する。
    fn record_download(&self, record: &FetchDownload) -> impl Future<Output = Result<()>> + Send;

    /// 取得失敗（403/404）を `failed` として記録する（#170 / ADR0024 論点1）。
    /// `attempts` を +1 し `http_status`/`last_attempt_at` を更新する。除外フラグではなく
    /// 再試行の入力。逐次 range の「連続成功直後の境界 403/404」だけが呼ぶ（ジャンク回避）。
    fn record_failure(&self, record: &FetchFailure) -> impl Future<Output = Result<()>> + Send;
}

/// netkeiba 由来の近走履歴の upsert と、pdf 成績への horse_id backfill。
pub trait HorseHistoryRepository: Send + Sync {
    /// netkeiba 由来の近走を horse 単位で `horses` / `horse_past_runs` に upsert する。
    /// pdf 確定成績(`results`)とは別テーブルに保存することで、集計の二重計上・フィールド
    /// バイアス（#58/#59）を構造的に防ぐ。`runs` が空のときは何もしない。
    /// 戻り値は upsert した近走数（canonical race_id へ変換できず skip した走は含まない。
    /// 冪等再取り込みでの ON CONFLICT 更新も「保存」として数えるため、初回取り込みでのみ
    /// 純粋な DB 行増分と一致する）。
    fn upsert_horse_history(
        &self,
        horse_id: &HorseId,
        runs: &[HorsePastRun],
    ) -> impl Future<Output = Result<usize>> + Send;

    /// `horses` マスタに馬名がちょうど 1 件一致する pdf 成績行（`results.horse_id IS NULL`）へ
    /// horse_id を backfill する。同名別馬（複数一致）・不一致は NULL のまま残し、既存値は
    /// 上書きしない（冪等）。埋めた行数を返す。
    fn backfill_results_horse_ids(&self) -> impl Future<Output = Result<u64>> + Send;
}

/// 予想セッション（収支・買い目・馬場入力）の読み書き。
pub trait PredictSessionRepository: Send + Sync {
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

    /// 指定日のセッションで購入済みの買い目を `(bet_id, レコード)` で bet_id 昇順に返す。
    /// 自動精算（#40）で payout を bet_id 指定で UPDATE するため、bet_id を併せて返す。
    fn find_predict_bets_with_id(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<Vec<(i64, PredictBetRecord)>>> + Send;

    /// 自動精算（#40）の書き込みを 1 トランザクションで行う。
    /// `settled` の各 `(bet_id, payout)` で `predict_bets.payout` を UPDATE し、
    /// セッションヘッダ（残高・累計・completed・updated_at）を upsert する。
    fn settle_predict_session(
        &self,
        session: &PredictSessionRecord,
        settled: &[(i64, u64)],
    ) -> impl Future<Output = Result<()>> + Send;

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

    /// 指定日のセッションで記録済みの馬場入力を返す（`--resume` 時のデフォルト提示用）。
    /// `track_condition` が `None` の行は「不明として入力済み」を表す。
    fn find_predict_race_conditions(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<Vec<PredictRaceConditionRecord>>> + Send;

    /// 1 レース分の馬場入力を upsert する。買い目の有無に依存せず入力直後に記録するため、
    /// セッション更新（`save_race_outcome`）とは独立に呼ぶ。`date` はセッション
    /// （`predict_sessions.date`）への FK キーで、レコード本体（`race_id`/`track_condition`）
    /// とは別管理とする。`recorded_at` は use-case 層が注入し gateway を時計から独立に保つ
    /// （[`FetchRecord`] と同じ流儀）。
    fn save_predict_race_condition(
        &self,
        date: NaiveDate,
        record: &PredictRaceConditionRecord,
        recorded_at: DateTime<Utc>,
    ) -> impl Future<Output = Result<()>> + Send;
}

/// pad 予想（印・短評・買い目・結果）の保存・取得。
pub trait PadPredictionRepository: Send + Sync {
    /// 予想（印・短評・買い目・結果）を保存する。`(date, venue, race_num)` で upsert し、
    /// 馬・買い目の子行は入れ替え（delete→insert）で冪等にする。`race_id` は実装側で
    /// `races`/`race_cards` を `(date, venue, race_num)` 照合し解決できた時のみ格納する。
    /// `now` は use-case 層が注入し gateway を時計から独立に保つ。
    fn save_pad_prediction(
        &self,
        prediction: &PadPrediction,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// 1 レース分の予想を返す（未保存なら `None`）。
    fn find_pad_prediction(
        &self,
        date: NaiveDate,
        venue: Venue,
        race_num: u32,
    ) -> impl Future<Output = Result<Option<PadPrediction>>> + Send;

    /// 保存済みの全予想を date / venue / race_num 昇順で返す（生成・検証用）。
    fn list_pad_predictions(&self) -> impl Future<Output = Result<Vec<PadPrediction>>> + Send;

    /// 予想を横断検索する（#145）。`filter` の指定軸のみ AND で絞り、`date DESC, venue, race_num`
    /// 昇順・`limit`/`offset` でページングしたサマリと、フィルタ適用後の総件数を返す。
    fn search_predictions(
        &self,
        filter: &PredictionFilter,
    ) -> impl Future<Output = Result<PredictionSearchResult>> + Send;

    /// 予想 1 件を主キー（`prediction_id`）で返す（未存在なら `None`）。個別予想ビュー用。
    fn find_pad_prediction_by_id(
        &self,
        prediction_id: i64,
    ) -> impl Future<Output = Result<Option<PadPrediction>>> + Send;

    /// 印別の的中率集計を返す（#145）。母集団は結果記録済みの予想で、`filter` で期間・場を絞れる。
    fn prediction_mark_stats(
        &self,
        filter: &MarkStatsFilter,
    ) -> impl Future<Output = Result<Vec<MarkStatRow>>> + Send;
}

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
{
}
