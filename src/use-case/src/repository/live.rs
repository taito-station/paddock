use core::future::Future;

use chrono::NaiveDate;

use crate::error::Result;

/// ライブ EV 監視サイクルの評価結果 1 行（`live_ev_snapshots` の 1 レコード）。
/// フリップ算出のため 1 レースにつき最新（`rank=1`）と直前（`rank=2`）の 2 行までをフラットに運び、
/// 最新/直前へのグルーピングは interactor が行う（`analyze` の Row 流儀）。`slip_json` は
/// JSONB 列 `slip` を JSON テキストのまま運び、rest-controller の DTO でデシリアライズする
/// （use-case を serde 非依存に保つ）。日付・時刻は DB の TEXT 規約どおり文字列で運ぶ。
#[derive(Debug, Clone)]
pub struct LiveEvSnapshot {
    /// 1 = 最新サイクル、2 = 直前サイクル（`captured_at` 降順の順位）。
    pub rank: u32,
    pub race_id: String,
    pub venue: String,
    pub race_no: u32,
    pub post_time: Option<String>,
    /// 監視サイクル時刻（UTC rfc3339。辞書順＝時刻順）。
    pub captured_at: String,
    /// `'bet'`（ROI≥100%）/ `'skip'`（−EV）。
    pub verdict: String,
    /// 全 3 券種 ROI[%]。
    pub roi: f64,
    /// 荒れ度（純モデル勝率分布の正規化エントロピー [0,1]。#344）。本 migration 以前の行は None。
    pub roughness: Option<f64>,
    pub konsen: bool,
    /// ◎馬番（model 勝率最上位）。
    pub axis: u32,
    /// ◎の model 勝率[%]。
    pub axis_prob: f64,
    /// ◎の単勝オッズ（欠落時 None）。
    pub axis_win_odds: Option<f64>,
    /// ◎の複勝オッズ下限（帯 low。欠落時 None）。#346
    pub axis_place_odds_low: Option<f64>,
    /// ◎の複勝オッズ上限（帯 high。欠落時 None）。#346
    pub axis_place_odds_high: Option<f64>,
    /// 賭金が乗っているのにオッズ未取得の脚があるか（#631）。**「ROI が過小評価される」ではない**
    /// ——`roi` は priced 脚だけで分子・分母とも算出される（式は対称）。true のとき、`roi` と
    /// 賭け計が**別の母集団**を指すという意味。
    pub odds_missing: bool,
    /// 買い目伝票 JSONB（`slip` 列）の JSON テキスト。
    pub slip_json: String,
}

/// ライブ EV 監視サイクルの評価結果 1 レコードの書き込み DTO（#346 / ADR 0064）。
/// predict-watch が 1 スイープ 1 レースを評価するたびに best-effort で upsert する。
/// use-case を serde 非依存に保つため、slip 伝票は構造化 `Vec<SlipLegRecord>` で運び、
/// JSONB 化（`slip` / `raw` 列）は gateway が行う。`(race_id, captured_at)` で冪等 upsert する。
#[derive(Debug, Clone)]
pub struct LiveEvSnapshotRecord {
    /// 開催日（`live_ev_snapshots.date` TEXT `'YYYY-MM-DD'`）。
    pub date: NaiveDate,
    pub race_id: String,
    /// 開催場 slug（例 `"tokyo"`。SPA が JP へ写像するため slug で保存する）。
    pub venue: String,
    pub race_no: u32,
    /// 発走時刻（`HH:MM`。race_card 由来。欠落時 None）。
    pub post_time: Option<String>,
    /// 監視サイクル境界時刻（UTC rfc3339 秒精度 Z 終端。1 スイープ 1 値・辞書順＝時刻順）。
    pub captured_at: String,
    /// `'bet'`（参考 ROI≥ゲート）/ `'skip'`。
    pub verdict: String,
    /// 全 3 券種 ROI[%]。
    pub roi: f64,
    /// 荒れ度（純モデル勝率分布の正規化エントロピー [0,1]。0=堅い〜1=荒れ。#344）。ROI とは別軸。
    pub roughness: f64,
    pub konsen: bool,
    /// ◎馬番（model 勝率最上位）。
    pub axis: u32,
    /// ◎の model 勝率[%]。
    pub axis_prob: f64,
    /// ◎の単勝オッズ（欠落時 None）。
    pub axis_win_odds: Option<f64>,
    /// ◎の複勝オッズ帯 low / high（欠落時 None。#346）。
    pub axis_place_odds_low: Option<f64>,
    pub axis_place_odds_high: Option<f64>,
    /// 賭金が乗っているのにオッズ未取得の脚があるか（#631）。**「ROI が過小評価される」ではない**
    /// ——`roi` は priced 脚だけで分子・分母とも算出される（式は対称）。true のとき、`roi` と
    /// 賭け計が**別の母集団**を指すという意味。
    pub odds_missing: bool,
    /// このレースに配分した予算（円）。
    pub race_budget: u64,
    /// 買い目伝票の leg（`1 leg = 1 組番 = 1 点` 粒度。SPA 側で券種×方式に再グルーピングされる）。
    pub legs: Vec<SlipLegRecord>,
}

/// 買い目伝票の 1 leg（式別×方式×軸×組番×点数×金額の「そのまま買える形」）。
/// `schema::live::SlipLeg` と同一契約で、gateway が `slip` JSONB へ直列化する。
#[derive(Debug, Clone)]
pub struct SlipLegRecord {
    /// 式別（`wide` / `quinella` / `trio` 等の安定ラベル）。
    pub bet_type: String,
    /// 方式（`nagashi` / `box` / `formation`）。
    pub method: String,
    /// ◎馬番（`method=box` では None）。
    pub axis: Option<u32>,
    /// 組番（昇順ソート済み）。
    pub combo: Vec<u32>,
    /// この leg の点数（emit 粒度では常に 1）。
    pub points: u32,
    /// 金額（100 円単位）。
    pub amount: u64,
}

/// その日の**初回スイープ**で確定した買い目選定（#601 軸ロック）。
///
/// `predict-watch` はスイープのたびに軸・相手・混戦判定を `rank_probs`（市場ブレンド α=0.2）から
/// 選び直すため、固定しないと選定がオッズに追随して動く（実測 154R 中 軸 28R・相手 62R）。
/// 2 スイープ目以降はこの pin を `PortfolioConfig` の固定フィールドへ渡して買い目を据え置き、
/// オッズで動かすのは EV/ROI と金額だけにする（CLAUDE.md 軸ロック・REQ-D01-003 / ADR 0060）。
///
/// **プロセスの再起動や `--once`（cron）を跨いでも効く**ように、in-memory ではなく
/// 既にアーカイブしている `live_ev_snapshots` から読み戻す（マイグレーション不要）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveEvPin {
    pub race_id: String,
    /// 初回スイープの軸（◎）。
    pub axis: u32,
    /// 初回スイープの相手。馬番昇順。
    pub partners: Vec<u32>,
    /// 初回スイープの混戦 band（印馬）。**空 `Vec` は「非混戦だった」を意味する**
    /// （`Option` にすると「未取得」と区別できず、混戦の反転を止められない）。
    pub konsen_band: Vec<u32>,
    /// 初回スイープの時刻（UTC rfc3339）。固定を適用した旨をログに出すために持つ。
    pub captured_at: String,
}

/// ライブ EV スナップショット（`live_ev_snapshots`, #260 / ADR 0064）の取得・書き込み。
/// 書き込みは #346 で Rust（predict-watch）に一本化した。旧 Python writer（`persist_live_ev.py` /
/// `refresh_ev.sh` の永続化ステップ）は #346 PR-3 で退役済み。`live_ev.py` 本体はオフライン用途で温存。
pub trait LiveEvRepository: Send + Sync {
    /// 指定開催日の全 race について、`captured_at` 降順で最新 2 サイクル（`rank<=2`）を
    /// フラットに返す。並びは `(race_id, rank)` 昇順。該当行が無ければ空 `Vec`。
    fn find_live_ev_by_date(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<Vec<LiveEvSnapshot>>> + Send;

    /// 指定開催日の race ごとに **`captured_at` 最古**（＝その日の初回スイープ）の選定を返す（#601）。
    /// 該当行が無ければ空 `Vec`（＝まだ 1 度も評価していない＝固定なし）。
    /// [`find_live_ev_by_date`](Self::find_live_ev_by_date) は最新 2 件を返す別用途なので混同しない。
    fn find_live_ev_pins_by_date(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<Vec<LiveEvPin>>> + Send;

    /// ライブ EV スナップショット 1 レコードを upsert する（`(race_id, captured_at)` 冪等）。
    /// 監視ループから best-effort で呼ぶため、失敗は呼び出し側が握って監視を継続する。
    fn save_live_ev_snapshot(
        &self,
        record: &LiveEvSnapshotRecord,
    ) -> impl Future<Output = Result<()>> + Send;
}
