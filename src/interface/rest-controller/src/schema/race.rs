use chrono::NaiveDate;
use serde::Serialize;
use utoipa::ToSchema;

use paddock_domain::{BetCombination, HorseEntry, HorseProbability, Portfolio, Race, RaceCard};

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

/// 推奨ポートフォリオ内の 1 買い目。
#[derive(Debug, Serialize, ToSchema)]
pub struct RecommendationBet {
    /// 券種ラベル（`馬連` / `ワイド` / `三連複`）。
    pub bet_type: String,
    /// 組合せキー（馬連・ワイドは `a-b`、三連複は `a-b-c`）。
    pub combination: String,
    /// 賭け金（円, 100 円単位）。
    pub stake: u64,
    /// 払戻倍率（保存オッズ未取得の脚は `null`）。ワイドは下限/上限帯の中点。
    pub odds: Option<f64>,
    /// 期待値倍率（`simulate` 単体評価）。odds 未取得なら 0.0。
    pub ev: f64,
}

/// `GET /api/races/{race_id}/recommendations` のレスポンス。
///
/// CLI `predict` と同じ軸流しポートフォリオ（馬連＋ワイド＋三連複, #122）を予算内・100 円単位で
/// 返す。`odds_available=false` は保存オッズ（#51）が無いことを示し、買い目は空（SPA は「最新取得」
/// を促す）。
#[derive(Debug, Serialize, ToSchema)]
pub struct RecommendationResponse {
    pub race_id: String,
    /// 保存オッズ（#51）の有無。false のとき `bets` は空。
    pub odds_available: bool,
    /// 軸（予想本命）の馬番。確率が空なら `null`。
    pub axis: Option<u32>,
    /// 相手（流す先）の馬番。
    pub partners: Vec<u32>,
    pub bets: Vec<RecommendationBet>,
    pub total_stake: u64,
    /// オッズ取得済みの脚に基づく期待回収率（倍率）。買い目が空なら `null`。
    pub roi: Option<f64>,
    /// 同上の的中確率 [0,1]。
    pub hit_prob: Option<f64>,
}

impl RecommendationResponse {
    /// 保存オッズが無いレースの応答（買い目なし）。
    pub fn odds_unavailable(race_id: String) -> Self {
        Self {
            race_id,
            odds_available: false,
            axis: None,
            partners: Vec::new(),
            bets: Vec::new(),
            total_stake: 0,
            roi: None,
            hit_prob: None,
        }
    }

    /// 生成済みポートフォリオから応答を組む。
    pub fn from_portfolio(race_id: String, p: Portfolio) -> Self {
        Self {
            race_id,
            odds_available: true,
            axis: p.axis.map(|h| h.value()),
            partners: p.partners.iter().map(|h| h.value()).collect(),
            bets: p
                .bets
                .iter()
                .map(|b| {
                    let (bet_type, combination) = combination_parts(&b.combination);
                    RecommendationBet {
                        bet_type: bet_type.to_string(),
                        combination,
                        stake: b.stake,
                        odds: b.odds,
                        ev: b.ev,
                    }
                })
                .collect(),
            total_stake: p.total_stake,
            roi: p.ev.as_ref().map(|e| e.roi),
            hit_prob: p.ev.as_ref().map(|e| e.hit_prob),
        }
    }
}

/// `BetCombination` を券種ラベルと組合せキー文字列に分解する。
/// ペア（馬連・ワイド・馬単）と三連系のキーは `find_race_odds` の `from_key` と対称。
fn combination_parts(c: &BetCombination) -> (&'static str, String) {
    match c {
        BetCombination::Win(h) => ("単勝", h.value().to_string()),
        BetCombination::Place(h) => ("複勝", h.value().to_string()),
        BetCombination::Quinella(p) => {
            let (a, b) = p.as_tuple();
            ("馬連", format!("{}-{}", a.value(), b.value()))
        }
        BetCombination::Wide(p) => {
            let (a, b) = p.as_tuple();
            ("ワイド", format!("{}-{}", a.value(), b.value()))
        }
        BetCombination::Exacta(p) => {
            let (a, b) = p.as_tuple();
            ("馬単", format!("{}>{}", a.value(), b.value()))
        }
        BetCombination::Trio(t) => {
            let (a, b, c) = t.as_tuple();
            (
                "三連複",
                format!("{}-{}-{}", a.value(), b.value(), c.value()),
            )
        }
        BetCombination::Trifecta(t) => {
            let (a, b, c) = t.as_tuple();
            (
                "三連単",
                format!("{}>{}>{}", a.value(), b.value(), c.value()),
            )
        }
    }
}
