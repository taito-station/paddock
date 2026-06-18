use chrono::NaiveDate;
use serde::Serialize;
use utoipa::ToSchema;

use paddock_domain::{HorseEntry, HorseProbability, Race, RaceCard};

/// レース一覧の 1 要素（出走前の諸元のみ。results は含まない）。
#[derive(Debug, Serialize, ToSchema)]
pub struct RaceSummary {
    /// レース ID（`RaceId` の文字列表現）。
    pub race_id: String,
    /// 開催日。
    pub date: NaiveDate,
    /// 開催場（英字スラッグ。例 `nakayama`）。
    pub venue: String,
    /// レース番号（1..=12）。
    pub race_num: u32,
    /// 距離[m]。
    pub distance: u32,
    /// 芝/ダート（`turf` / `dirt`）。
    pub surface: String,
}

impl From<&Race> for RaceSummary {
    fn from(r: &Race) -> Self {
        Self {
            race_id: r.race_id.value().to_string(),
            date: r.date,
            venue: r.venue.as_slug().to_string(),
            race_num: r.race_num,
            distance: r.distance,
            surface: r.surface.as_str().to_string(),
        }
    }
}

/// `GET /api/races?date=` のレスポンス。
#[derive(Debug, Serialize, ToSchema)]
pub struct RaceListResponse {
    pub date: NaiveDate,
    pub races: Vec<RaceSummary>,
}

/// 出馬表の 1 頭。
#[derive(Debug, Serialize, ToSchema)]
pub struct HorseEntrySchema {
    /// 枠番（1..=8）。
    pub gate_num: u32,
    /// 馬番。
    pub horse_num: u32,
    pub horse_name: String,
    /// 騎手名（PDF 出馬表由来は `null`）。
    pub jockey: Option<String>,
    /// 調教師名（PDF 出馬表由来は `null`）。
    pub trainer: Option<String>,
    /// 負担重量[kg]（PDF 出馬表由来は `null`）。
    pub weight_carried: Option<f64>,
}

impl From<&HorseEntry> for HorseEntrySchema {
    fn from(e: &HorseEntry) -> Self {
        Self {
            gate_num: e.gate_num.value(),
            horse_num: e.horse_num.value(),
            horse_name: e.horse_name.value().to_string(),
            jockey: e.jockey.as_ref().map(|j| j.value().to_string()),
            trainer: e.trainer.as_ref().map(|t| t.value().to_string()),
            weight_carried: e.weight_carried,
        }
    }
}

/// `GET /api/races/{race_id}` のレスポンス（出馬表）。
#[derive(Debug, Serialize, ToSchema)]
pub struct RaceCardResponse {
    pub race_id: String,
    pub date: NaiveDate,
    pub venue: String,
    pub round: u32,
    pub day: u32,
    pub race_num: u32,
    pub surface: String,
    pub distance: u32,
    pub entries: Vec<HorseEntrySchema>,
}

impl From<RaceCard> for RaceCardResponse {
    fn from(c: RaceCard) -> Self {
        Self {
            race_id: c.race_id.value().to_string(),
            date: c.date,
            venue: c.venue.as_slug().to_string(),
            round: c.round,
            day: c.day,
            race_num: c.race_num,
            surface: c.surface.as_str().to_string(),
            distance: c.distance,
            entries: c.entries.iter().map(HorseEntrySchema::from).collect(),
        }
    }
}

/// 1 頭分の win/place/show 確率。
#[derive(Debug, Serialize, ToSchema)]
pub struct HorseProbabilitySchema {
    pub horse_num: u32,
    pub horse_name: String,
    /// 勝率 [0,1]。
    pub win_prob: f64,
    /// 連対率（2 着以内）[0,1]。
    pub place_prob: f64,
    /// 複勝率（3 着以内）[0,1]。`win_prob ≤ place_prob ≤ show_prob` を保証。
    pub show_prob: f64,
}

impl From<&HorseProbability> for HorseProbabilitySchema {
    fn from(p: &HorseProbability) -> Self {
        Self {
            horse_num: p.horse_num.value(),
            horse_name: p.horse_name.value().to_string(),
            win_prob: p.win_prob,
            place_prob: p.place_prob,
            show_prob: p.show_prob,
        }
    }
}

/// `GET /api/races/{race_id}/prediction` のレスポンス。
#[derive(Debug, Serialize, ToSchema)]
pub struct PredictionResponse {
    pub race_id: String,
    pub probabilities: Vec<HorseProbabilitySchema>,
}
