use core::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::{Mark, PadPrediction, RaceId, Surface, TrackCondition, Venue};

use crate::error::Result;
use crate::repository::stats::{MarkStatRow, MarkStatsFilter};

/// 予想セッション 1 件（1 開催日 = 1 セッション）。途中離脱後の `--resume` と
/// 収支サマリ `--summary` のために永続化する。`created_at`/`updated_at` は use-case 層が
/// 時刻を注入し、gateway を時計から独立に保つ（[`crate::repository::FetchRecord`] と同じ流儀）。
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

    /// 1 レース分の確定結果を 1 トランザクションでアトミックに記録する（#469）。
    ///
    /// tx 冒頭で `SELECT ... FROM predict_sessions WHERE date = $1 FOR UPDATE` により対象
    /// セッション行をロックし、二重記録ガード（当該レースの記録済み買い目の有無）・残高ガード
    /// （`Σstake ≤ balance`）・残高/累計計算・セッション upsert・買い目追記を **すべてロック下で**
    /// 行う。これにより同時 POST／リトライでの買い目重複＋残高二重適用（TOCTOU）を防ぐ。
    ///
    /// - セッション未作成: `NotFound`
    /// - 当該レースへ買い目ありで再記録: `Conflict`（買い目なしの再 POST はスキップの冪等再送として許容）
    /// - `Σstake > balance`: `InvalidArgument`（状態不変）
    ///
    /// 成功時は更新後のセッションレコードを返す。`updated_at` として `now` を注入する。
    fn save_race_outcome(
        &self,
        date: NaiveDate,
        race_id: &RaceId,
        bets: &[PredictBetRecord],
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<PredictSessionRecord>> + Send;

    /// 指定日のセッションで「見送り（スキップ）」として記録済みのレース ID を返す（#481）。
    /// 買い目ありで記録されたレースは `find_predict_bets` 側に現れるため、ここには含まれない。
    /// web 盤の再訪時に「見送り済み」バッジを出す判定に使う。
    fn find_predict_race_skips(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<Vec<RaceId>>> + Send;

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
    /// （[`crate::repository::FetchRecord`] と同じ流儀）。
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
