use core::future::Future;

use chrono::{DateTime, Utc};
use paddock_domain::HorseId;

use crate::error::Result;
use crate::netkeiba_scraper::HorsePastRun;

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
