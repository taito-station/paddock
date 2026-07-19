use std::collections::HashMap;

use chrono::{NaiveDate, NaiveTime};
use serde::Serialize;
use utoipa::ToSchema;

use paddock_domain::{
    BetCombination, HorseEntry, HorseProbability, Portfolio, Race, RaceCard, RaceId,
};
use paddock_use_case::{FinishEntry, RaceBoard};

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
    /// 発走時刻（`HH:MM`、race_cards 由来。未保存なら `null`）。
    /// ライブ一覧の状態判定（未発走/終了）の一次ソース（#391）。
    pub post_time: Option<String>,
    /// 表示用レース名（race_cards 由来。重賞・特別戦名。未保存/PDF 経路なら `null`。#389）。
    pub race_name: Option<String>,
    /// 結果確定フラグ（#381。`results` に着順ありの行が 1 件以上）。web の「⚫終」判定を post_time
    /// 推定でなく着順確定で行うための一次ソース。未確定・未取得は false。
    pub result_confirmed: bool,
    /// 上位着順（#381。`finishing_position <= 3`・着順昇順。3 着同着で 4 件以上ありうる＝件数可変）。
    /// 未確定レースは空配列。
    pub finish_order: Vec<FinishEntrySchema>,
}

/// レース結果の上位着順 1 行（#381）。ライブ一覧の着順表示に使う。
#[derive(Debug, Serialize, ToSchema)]
pub struct FinishEntrySchema {
    /// 着順（1..）。3 着同着なら同じ position が複数行。
    pub position: u32,
    pub horse_num: u32,
    pub horse_name: String,
}

impl RaceSummary {
    /// レース諸元＋発走時刻・レース名・結果確定/上位着順（`race_id → …` マップから引き当て）で組み立てる。
    pub fn new(
        r: &Race,
        post_times: &HashMap<RaceId, NaiveTime>,
        race_names: &HashMap<RaceId, String>,
        confirmed: &HashMap<RaceId, bool>,
        finishes: &HashMap<RaceId, Vec<FinishEntry>>,
    ) -> Self {
        Self {
            race_id: r.race_id.value().to_string(),
            date: r.date,
            venue: r.venue.as_slug().to_string(),
            race_num: r.race_num,
            distance: r.distance,
            surface: r.surface.as_str().to_string(),
            post_time: post_times
                .get(&r.race_id)
                .map(|t| t.format("%H:%M").to_string()),
            race_name: race_names.get(&r.race_id).cloned(),
            result_confirmed: confirmed.get(&r.race_id).copied().unwrap_or(false),
            finish_order: finishes
                .get(&r.race_id)
                .map(|v| {
                    v.iter()
                        .map(|f| FinishEntrySchema {
                            position: f.position,
                            horse_num: f.horse_num,
                            horse_name: f.horse_name.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
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
    /// 表示用レース名（race_cards 由来。重賞・特別戦名。未保存/PDF 経路なら `null`。#389）。
    pub race_name: Option<String>,
    /// 格付けスラッグ（`g1`/`g2`/`g3`/`listed`/`open`/`win3`… 由来 #345。未判定/PDF 経路なら `null`）。
    pub race_class: Option<String>,
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
            race_name: c.race_name,
            race_class: c.race_class.map(|rc| rc.as_str().to_string()),
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
    /// EV 視点（純モデル α=1.0・市場非依存）の勝率 [0,1]。連対/複勝は下記（#373 盤の3系統表示）。
    pub pure_win_prob: f64,
    /// 純モデル α=1.0 の連対率/複勝率 [0,1]（#373）。
    pub pure_place_prob: f64,
    pub pure_show_prob: f64,
    /// 市場implied 勝率（フィールド内 `1/単勝` 正規化。単勝未取得なら `null`）。
    pub market_implied: Option<f64>,
    pub win_odds: Option<f64>,
    /// 朝時点（最初にフル盤成立した snapshot）の単勝オッズ（#448）。`win_odds` との差で「▲人気化／△妙味」を出す。
    /// 朝 snapshot が無い（`morning_at=null`）・当該馬が朝未取得なら `null`。
    pub morning_win_odds: Option<f64>,
    pub place_odds_low: Option<f64>,
    pub place_odds_high: Option<f64>,
    /// 単勝人気（1=1番人気。単勝未取得なら `null`）。乖離判定の市場順位も兼ねる。
    pub popularity: Option<u32>,
    /// モデル勝率順位（1=最上位）。
    pub model_rank: u32,
    /// 機械導出の印スラッグ（honmei/taikou/tanana/hoshi）。無印は `null`。
    pub mark: Option<String>,
    /// 重なり馬（モデル勝率1位 かつ 単勝人気1位＝ほぼ複勝圏サイン）。
    pub is_overlay: bool,
    /// 乖離馬（モデル上位×市場人気低＝妙味・ワイドボックス候補）。
    pub is_value: bool,
    /// 確定着順（#381。`results` 由来。未確定・除外/中止で着順なしは `null`）。
    pub finishing_position: Option<u32>,
    /// 馬書評の一行寸評（人手優先・無ければルールベース生成, #348）。特筆材料なしは `null`。
    pub comment: Option<String>,
    /// 展開パネル用の根拠 bullet（条件別 factor・枠 lift・近走・前走・斤量）。空配列＝根拠情報なし。
    pub detail_lines: Vec<String>,
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
    /// 表示用レース名（重賞・特別戦名。未保存/PDF 経路なら `null`。#389）。
    pub race_name: Option<String>,
    /// 格付けスラッグ（`g1`/`g2`/`g3`/`listed`/`open`… #345。盤ヘッダのグレード表記に使う。未判定は `null`）。
    pub race_class: Option<String>,
    /// 保存オッズ（#51）の有無。false のとき `bets` は必ず空。
    pub odds_available: bool,
    /// 買い目の軸。記録軸（`recorded_axis`）があればそれに固定、無ければライブ再計算（`live_axis`）。
    /// ただし保存オッズが無い（`odds_available=false`）ときは買い目が組めず `axis=null`
    /// （`recorded_axis` は残る＝盤の軸ロック表示はオッズ無しでも出る）。
    pub axis: Option<u32>,
    /// predict 記録済みの本命◎（軸ロックの正, #388）。未 predict・取消時は `null`。
    /// `axis` はこれがあればこれに一致する（買い目軸を記録軸に固定する）。
    pub recorded_axis: Option<u32>,
    /// ライブ再計算の軸＝市場ブレンド首位（機械◎）。`recorded_axis` と異なるとき UI は乖離警告を出す（#388）。
    pub live_axis: Option<u32>,
    pub partners: Vec<u32>,
    pub bets: Vec<RecommendationBet>,
    pub total_stake: u64,
    pub roi: Option<f64>,
    pub hit_prob: Option<f64>,
    pub confusion: ConfusionSchema,
    /// レース書評（混戦度・◎の狙いどころ・妙味）。人手優先・無ければルールベース生成（#348）。`null` 可。
    pub race_comment: Option<String>,
    /// 結果確定フラグ（#381。`results` に着順ありの行が 1 件以上）。web の「⚫終」判定に使う。
    pub result_confirmed: bool,
    /// 朝時点（最初にフル盤成立した snapshot）の取得時刻 RFC3339（#448）。朝 complete と最新が別時刻の
    /// レースで非 `null`（発走前が主用途だが、終了レースでも複数時点の完全 snapshot があれば出る）。
    /// UI はこれが非 `null` の時だけ「朝↔現比較」を出す（`null` は従来どおり現時点のみ）。
    pub morning_at: Option<String>,
    /// 現時点（最新スイープ）の取得時刻 RFC3339（#448）。`morning_at` と対。
    pub current_at: Option<String>,
    /// 朝時点オッズで再計算したポートフォリオ ROI（#448。確率・軸・budget は現時点と同一）。
    pub morning_roi: Option<f64>,
    /// 朝時点オッズで再計算したポートフォリオ的中確率（#448）。
    pub morning_hit_prob: Option<f64>,
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
            race_name: b.race_name,
            race_class: b.race_class,
            odds_available,
            axis,
            recorded_axis: b.recorded_axis,
            live_axis: b.live_axis,
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
            race_comment: b.race_comment,
            result_confirmed: b.result_confirmed,
            morning_at: b.morning_at,
            current_at: b.current_at,
            morning_roi: b.morning_roi,
            morning_hit_prob: b.morning_hit_prob,
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
                    pure_place_prob: h.pure_place_prob,
                    pure_show_prob: h.pure_show_prob,
                    market_implied: h.market_implied,
                    win_odds: h.win_odds,
                    morning_win_odds: h.morning_win_odds,
                    place_odds_low: h.place_odds_low,
                    place_odds_high: h.place_odds_high,
                    popularity: h.popularity,
                    model_rank: h.model_rank,
                    mark: h.mark,
                    is_overlay: h.is_overlay,
                    is_value: h.is_value,
                    finishing_position: h.finishing_position,
                    comment: h.comment,
                    detail_lines: h.detail_lines,
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
