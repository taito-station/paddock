use chrono::NaiveDate;
use serde::Serialize;
use utoipa::ToSchema;

use paddock_domain::PadPrediction;
use paddock_use_case::repository::{MarkStatRow, PredictionSummaryRow};

/// 検索一覧の 1 要素（サマリ）。馬・買い目の全量は持たず、個別取得で補う。
#[derive(Debug, Serialize, ToSchema)]
pub struct PredictionSummarySchema {
    pub prediction_id: i64,
    pub date: NaiveDate,
    /// 開催場（英字スラッグ。例 `nakayama`）。
    pub venue: String,
    pub race_num: u32,
    /// レース ID（`races`/`race_cards` 照合済みのときのみ。未照合は `null`）。
    pub race_id: Option<String>,
    pub title: Option<String>,
    /// 距離[m]（`races` 結合で得た値。未照合は `null`）。
    pub distance: Option<u32>,
    /// 芝/ダート（`turf`/`dirt`。未照合は `null`）。
    pub surface: Option<String>,
    /// 印 ◎ の馬名（◎が複数なら horse_num 昇順の先頭。無ければ `null`）。
    pub honmei_horse: Option<String>,
    /// `[finish_1, finish_2, finish_3]`（馬番。各要素は `null` 可）。結果未記録なら `null`。
    pub finish: Option<Vec<Option<u32>>>,
    pub recovery_rate: Option<f64>,
    pub pnl: Option<i64>,
    /// 的中。`recovery_rate > 0`→`true`、結果あり且つ払戻 0 以下→`false`、結果未記録→`null`。
    pub hit: Option<bool>,
}

impl From<PredictionSummaryRow> for PredictionSummarySchema {
    fn from(r: PredictionSummaryRow) -> Self {
        Self {
            prediction_id: r.prediction_id,
            date: r.date,
            venue: r.venue.as_slug().to_string(),
            race_num: r.race_num,
            race_id: r.race_id,
            title: r.title,
            distance: r.distance,
            surface: r.surface.map(|s| s.as_str().to_string()),
            honmei_horse: r.honmei_horse,
            finish: r.finish.map(|f| f.to_vec()),
            recovery_rate: r.recovery_rate,
            pnl: r.pnl,
            hit: r.hit,
        }
    }
}

/// `GET /api/predictions` のレスポンス。
#[derive(Debug, Serialize, ToSchema)]
pub struct PredictionSearchResponse {
    /// フィルタ適用後の総件数（ページング用）。
    pub total_count: u64,
    pub limit: u32,
    pub offset: u32,
    pub predictions: Vec<PredictionSummarySchema>,
}

/// 予想 1 頭分（印・確率・単勝/人気・短評）。
#[derive(Debug, Serialize, ToSchema)]
pub struct PredictionHorseSchema {
    pub horse_num: u32,
    pub horse_name: String,
    pub jockey: Option<String>,
    /// 印スラッグ（`honmei`/`taikou`/`tanana`/`renge`/`hoshi`/`chui`）。無印は `null`。
    pub mark: Option<String>,
    pub win_odds: Option<f64>,
    pub popularity: Option<u32>,
    pub win_prob: Option<f64>,
    pub place_prob: Option<f64>,
    pub show_prob: Option<f64>,
    pub comment: Option<String>,
}

/// 買い目 1 点。
#[derive(Debug, Serialize, ToSchema)]
pub struct PredictionBetSchema {
    /// 券種（`単勝`/`複勝`/`馬連`/`ワイド`/`馬単`/`3連複`/`3連単`）。
    pub bet_type: String,
    /// 組合せ（馬番のハイフン連結。例 `7` / `7-14` / `7-14-13`）。
    pub combination: String,
    pub amount: u64,
}

/// レース結果（答え合わせ後にのみ付く）。
#[derive(Debug, Serialize, ToSchema)]
pub struct PredictionResultSchema {
    /// `[finish_1, finish_2, finish_3]`（馬番。各要素は `null` 可）。
    pub finish: Vec<Option<u32>>,
    pub recovery_rate: Option<f64>,
    pub pnl: Option<i64>,
    pub note: Option<String>,
}

/// `GET /api/predictions/{prediction_id}` のレスポンス（個別予想・全項目）。
#[derive(Debug, Serialize, ToSchema)]
pub struct PadPredictionResponse {
    pub prediction_id: i64,
    pub date: NaiveDate,
    pub venue: String,
    pub race_num: u32,
    pub title: Option<String>,
    pub budget: Option<u64>,
    pub strategy_note: Option<String>,
    pub commentary: Option<String>,
    pub horses: Vec<PredictionHorseSchema>,
    pub bets: Vec<PredictionBetSchema>,
    pub result: Option<PredictionResultSchema>,
}

impl PadPredictionResponse {
    /// ドメインの [`PadPrediction`] と主キーから組み立てる（主キーはドメイン型が持たないため別引数）。
    pub fn from_domain(prediction_id: i64, p: PadPrediction) -> Self {
        Self {
            prediction_id,
            date: p.date,
            venue: p.venue.as_slug().to_string(),
            race_num: p.race_num,
            title: p.title,
            budget: p.budget,
            strategy_note: p.strategy_note,
            commentary: p.commentary,
            horses: p
                .horses
                .into_iter()
                .map(|h| PredictionHorseSchema {
                    horse_num: h.horse_num,
                    horse_name: h.horse_name,
                    jockey: h.jockey,
                    mark: h.mark.map(|m| m.as_slug().to_string()),
                    win_odds: h.win_odds,
                    popularity: h.popularity,
                    win_prob: h.win_prob,
                    place_prob: h.place_prob,
                    show_prob: h.show_prob,
                    comment: h.comment,
                })
                .collect(),
            bets: p
                .bets
                .into_iter()
                .map(|b| PredictionBetSchema {
                    bet_type: b.bet_type,
                    combination: b.combination,
                    amount: b.amount,
                })
                .collect(),
            result: p.result.map(|r| PredictionResultSchema {
                finish: r.finish.to_vec(),
                recovery_rate: r.recovery_rate,
                pnl: r.pnl,
                note: r.note,
            }),
        }
    }
}

/// 印 1 種の的中率。
#[derive(Debug, Serialize, ToSchema)]
pub struct MarkStatSchema {
    /// 印スラッグ（`honmei` 等）。
    pub mark: String,
    /// その印が付いた（結果記録済みの）馬の延べ数。
    pub count: u32,
    /// 1 着に入った延べ数。
    pub win: u32,
    /// 複勝圏（3 着内）に入った延べ数。
    pub show: u32,
    /// 1 着率（`win / count`）。
    pub win_rate: f64,
    /// 複勝圏到達率（`show / count`。予想入力の `show_prob` とは別概念）。
    pub show_rate: f64,
}

impl From<&MarkStatRow> for MarkStatSchema {
    fn from(r: &MarkStatRow) -> Self {
        Self {
            mark: r.mark.as_slug().to_string(),
            count: r.count,
            win: r.win,
            show: r.show,
            win_rate: r.win_rate(),
            show_rate: r.show_rate(),
        }
    }
}

/// `GET /api/predictions/stats/by-mark` のレスポンス。
#[derive(Debug, Serialize, ToSchema)]
pub struct MarkStatsResponse {
    pub by_mark: Vec<MarkStatSchema>,
}
