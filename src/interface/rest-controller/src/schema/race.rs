use chrono::NaiveDate;
use serde::Serialize;
use utoipa::ToSchema;

use paddock_domain::{BetCombination, HorseEntry, HorseProbability, Portfolio, Race, RaceCard};
use paddock_use_case::RaceBoard;

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
/// 返す。`bets` が空になる原因は 2 通りで、`odds_available` で区別する:
/// - `odds_available=false`: 保存オッズ（#51）が無い → SPA は「最新取得」を促す。
/// - `odds_available=true` かつ `bets` 空: オッズはあるが予算内で組める買い目が無い（または確率が
///   空）＝「該当なし」→ 「最新取得」は出さない。
#[derive(Debug, Serialize, ToSchema)]
pub struct RecommendationResponse {
    pub race_id: String,
    /// 保存オッズ（#51）の有無。false のとき `bets` は必ず空。true でも予算内で組めなければ空になりうる。
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

/// 混戦サマリ（CLAUDE.md の混戦判定を機械化した結果）。
#[derive(Debug, Serialize, ToSchema)]
pub struct ConfusionSchema {
    /// 混戦か（◎の勝率×0.70 以上が ◎含め 4 頭以上）。
    pub is_confused: bool,
    /// ◎（モデル勝率1位）の勝率 [0,1]。
    pub axis_win_prob: f64,
    /// 判定しきい値（= `axis_win_prob * 0.70`）。
    pub threshold: f64,
    /// しきい値以上の頭数（◎含む）。
    pub qualifying_count: u32,
}

/// 盤の 1 頭分（全頭 truncate せず返す）。
#[derive(Debug, Serialize, ToSchema)]
pub struct BoardHorseSchema {
    /// 枠番（出馬表に無ければ `null`）。
    pub gate_num: Option<u32>,
    pub horse_num: u32,
    pub horse_name: String,
    pub jockey: Option<String>,
    /// 表示用（市場ブレンド α）の勝率/連対率/複勝率 [0,1]。
    pub win_prob: f64,
    pub place_prob: f64,
    pub show_prob: f64,
    /// EV 視点（純モデル α=1.0）の勝率 [0,1]。
    pub pure_win_prob: f64,
    /// 市場implied 勝率（フィールド内 `1/単勝` 正規化。単勝未取得なら `null`）。
    pub market_implied: Option<f64>,
    pub win_odds: Option<f64>,
    pub place_odds_low: Option<f64>,
    pub place_odds_high: Option<f64>,
    /// 単勝人気（1=1番人気。単勝未取得なら `null`）。
    pub popularity: Option<u32>,
    /// モデル勝率順位（1=最上位）。
    pub model_rank: u32,
    /// 市場人気順位（= `popularity`）。
    pub market_rank: Option<u32>,
    /// 機械導出の印スラッグ（honmei/taikou/tanana/hoshi）。無印は `null`。
    pub mark: Option<String>,
    /// 重なり馬（モデル勝率1位 かつ 単勝人気1位＝ほぼ複勝圏サイン）。
    pub is_overlay: bool,
    /// 乖離馬（モデル上位×市場人気低＝妙味・ワイドボックス候補）。
    pub is_value: bool,
}

/// `GET /api/races/{race_id}/board` のレスポンス（1レース盤）。
///
/// 全出走馬 ＋ 買い目 ＋ 混戦/乖離/重なりを 1 レスポンスで返す。`horses` は truncate しない
/// （買い目の相手 top5 から漏れる市場人気馬も盤で見える）。買い目（`axis`/`partners`/`bets`/
/// `roi`/`hit_prob`）は `/recommendations` と同経路・同値で、保存オッズが無ければ `odds_available=false`。
#[derive(Debug, Serialize, ToSchema)]
pub struct RaceBoardResponse {
    pub race_id: String,
    pub date: NaiveDate,
    pub venue: String,
    pub race_num: u32,
    pub surface: String,
    pub distance: u32,
    pub field_size: u32,
    /// 発走時刻 `HH:MM`（未取得は `null`）。
    pub post_time: Option<String>,
    /// 保存オッズ（#51）の有無。false のとき `bets` は必ず空。
    pub odds_available: bool,
    pub axis: Option<u32>,
    pub partners: Vec<u32>,
    pub bets: Vec<RecommendationBet>,
    pub total_stake: u64,
    pub roi: Option<f64>,
    pub hit_prob: Option<f64>,
    pub confusion: ConfusionSchema,
    pub horses: Vec<BoardHorseSchema>,
}

impl From<RaceBoard> for RaceBoardResponse {
    fn from(b: RaceBoard) -> Self {
        let (odds_available, axis, partners, bets, total_stake, roi, hit_prob) = match b.portfolio {
            Some(p) => (
                true,
                p.axis.map(|h| h.value()),
                p.partners.iter().map(|h| h.value()).collect(),
                p.bets
                    .iter()
                    .map(|bet| {
                        let (bet_type, combination) = combination_parts(&bet.combination);
                        RecommendationBet {
                            bet_type: bet_type.to_string(),
                            combination,
                            stake: bet.stake,
                            odds: bet.odds,
                            ev: bet.ev,
                        }
                    })
                    .collect(),
                p.total_stake,
                p.ev.as_ref().map(|e| e.roi),
                p.ev.as_ref().map(|e| e.hit_prob),
            ),
            None => (false, None, Vec::new(), Vec::new(), 0, None, None),
        };
        Self {
            race_id: b.race_id.value().to_string(),
            date: b.date,
            venue: b.venue,
            race_num: b.race_num,
            surface: b.surface,
            distance: b.distance,
            field_size: b.field_size,
            post_time: b.post_time,
            odds_available,
            axis,
            partners,
            bets,
            total_stake,
            roi,
            hit_prob,
            confusion: ConfusionSchema {
                is_confused: b.confusion.is_confused,
                axis_win_prob: b.confusion.axis_win_prob,
                threshold: b.confusion.threshold,
                qualifying_count: b.confusion.qualifying_count,
            },
            horses: b
                .horses
                .into_iter()
                .map(|h| BoardHorseSchema {
                    gate_num: h.gate_num,
                    horse_num: h.horse_num,
                    horse_name: h.horse_name,
                    jockey: h.jockey,
                    win_prob: h.win_prob,
                    place_prob: h.place_prob,
                    show_prob: h.show_prob,
                    pure_win_prob: h.pure_win_prob,
                    market_implied: h.market_implied,
                    win_odds: h.win_odds,
                    place_odds_low: h.place_odds_low,
                    place_odds_high: h.place_odds_high,
                    popularity: h.popularity,
                    model_rank: h.model_rank,
                    market_rank: h.market_rank,
                    mark: h.mark,
                    is_overlay: h.is_overlay,
                    is_value: h.is_value,
                })
                .collect(),
        }
    }
}

/// `BetCombination` を券種ラベルと組合せキー文字列に分解する。組合せキーは各ドメイン型の
/// `to_key()` に委譲し（`find_race_odds` の `from_key` と対称・形式の二重定義を避ける）、
/// 単勝・複勝の単一馬番はそのまま文字列化する。
fn combination_parts(c: &BetCombination) -> (&'static str, String) {
    match c {
        BetCombination::Win(h) => ("単勝", h.value().to_string()),
        BetCombination::Place(h) => ("複勝", h.value().to_string()),
        BetCombination::Quinella(p) => ("馬連", p.to_key()),
        BetCombination::Wide(p) => ("ワイド", p.to_key()),
        BetCombination::Exacta(p) => ("馬単", p.to_key()),
        BetCombination::Trio(t) => ("三連複", t.to_key()),
        BetCombination::Trifecta(t) => ("三連単", t.to_key()),
    }
}
