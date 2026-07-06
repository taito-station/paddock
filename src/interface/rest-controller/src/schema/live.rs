use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use paddock_use_case::{
    LiveFlip as LiveFlipView, LiveRaceView, LiveSummary as LiveSummaryView, LiveView,
};

/// `GET /api/live/{date}` のレスポンス（#260 / ADR 0064）。
/// 指定開催日の race ごと最新サイクル＋伝票＋フリップを返す（read-only）。
#[derive(Debug, Serialize, ToSchema)]
pub struct LiveResponse {
    /// 開催日（`YYYY-MM-DD`）。
    pub date: String,
    pub summary: LiveSummary,
    pub races: Vec<LiveRaceViewSchema>,
}

/// 一望サマリ（張る本数・監視数・最終更新時刻）。
#[derive(Debug, Serialize, ToSchema)]
pub struct LiveSummary {
    /// 最新サイクルが `verdict='bet'` の race 数。
    pub bet_race_count: u32,
    /// 監視レース数（= `races.len()`）。
    pub watched_race_count: u32,
    /// 全 race 中の最新 `captured_at` の最大値。無ければ null。
    pub last_updated: Option<String>,
}

/// 1 レースの最新サイクル本体＋伝票＋フリップ。
#[derive(Debug, Serialize, ToSchema)]
pub struct LiveRaceViewSchema {
    pub race_id: String,
    pub venue: String,
    pub race_no: u32,
    /// 発走時刻（netkeiba 由来文字列。欠落時 null）。
    pub post_time: Option<String>,
    /// 監視サイクル時刻（UTC rfc3339）。
    pub captured_at: String,
    /// `'bet'`（ROI≥100%）/ `'skip'`（−EV）。
    pub verdict: String,
    /// 全 3 券種 ROI[%]。
    pub roi: f64,
    pub konsen: bool,
    /// ◎馬番。
    pub axis: u32,
    /// ◎の model 勝率[%]。
    pub axis_prob: f64,
    /// ◎の単勝オッズ（欠落時 null）。
    pub axis_win_odds: Option<f64>,
    /// ◎の複勝オッズ下限（帯 low。欠落時 null）。
    pub axis_place_odds_low: Option<f64>,
    /// ◎の複勝オッズ上限（帯 high。欠落時 null）。
    pub axis_place_odds_high: Option<f64>,
    /// 一部買い目のオッズ欠落（ROI 過小評価の可能性）。
    pub odds_missing: bool,
    pub slip: SlipView,
    pub flip: LiveFlip,
}

/// 買い目伝票。`slip` JSONB 列（`{ race_budget, legs }`）をデシリアライズしたもの。
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SlipView {
    /// このレースに配分した予算（円）。
    pub race_budget: u64,
    /// (方式レイヤー × 券種) 単位の leg 配列。
    pub legs: Vec<SlipLeg>,
}

/// 伝票の 1 leg（式別×方式×軸×組番×点数×金額の「そのまま買える形」）。
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SlipLeg {
    /// 式別（`wide` / `quinella` / `trio`）。
    pub bet_type: String,
    /// 方式（`nagashi` / `box` / `formation`）。
    pub method: String,
    /// ◎馬番（`method=box` では null）。
    pub axis: Option<u32>,
    /// 組番（昇順ソート済み）。
    pub combo: Vec<u32>,
    /// この leg の点数。
    pub points: u32,
    /// 金額（100 円単位）。
    pub amount: u64,
}

/// 直前サイクルとの差分（◎変化・+EV↔−EV 反転）。直前が無ければ全て false / null。
#[derive(Debug, Serialize, ToSchema)]
pub struct LiveFlip {
    /// ◎馬番が直前から変化したか。
    pub axis_changed: bool,
    /// 直前サイクルの◎馬番（無ければ null）。
    pub prev_axis: Option<u32>,
    /// verdict が直前から反転（+EV↔−EV）したか。
    pub ev_reversed: bool,
    /// 直前サイクルの verdict（無ければ null）。
    pub prev_verdict: Option<String>,
    /// 直前サイクルの ROI[%]（無ければ null）。
    pub prev_roi: Option<f64>,
}

impl LiveResponse {
    /// use-case の [`LiveView`] を API レスポンスへ写像する。slip 伝票は JSON テキストで
    /// 運ばれるため、ここで [`SlipView`] にデシリアライズする（不正 JSON は永続化側の不整合
    /// なので `Err` を返し、handler が 500 に倒す）。
    pub fn from_view(view: LiveView) -> serde_json::Result<Self> {
        let races = view
            .races
            .into_iter()
            .map(LiveRaceViewSchema::from_view)
            .collect::<serde_json::Result<Vec<_>>>()?;
        Ok(Self {
            date: view.date.format("%Y-%m-%d").to_string(),
            summary: LiveSummary::from(view.summary),
            races,
        })
    }
}

impl From<LiveSummaryView> for LiveSummary {
    fn from(s: LiveSummaryView) -> Self {
        Self {
            bet_race_count: s.bet_race_count,
            watched_race_count: s.watched_race_count,
            last_updated: s.last_updated,
        }
    }
}

impl LiveRaceViewSchema {
    fn from_view(r: LiveRaceView) -> serde_json::Result<Self> {
        let slip: SlipView = serde_json::from_str(&r.slip_json)?;
        Ok(Self {
            race_id: r.race_id,
            venue: r.venue,
            race_no: r.race_no,
            post_time: r.post_time,
            captured_at: r.captured_at,
            verdict: r.verdict,
            roi: r.roi,
            konsen: r.konsen,
            axis: r.axis,
            axis_prob: r.axis_prob,
            axis_win_odds: r.axis_win_odds,
            axis_place_odds_low: r.axis_place_odds_low,
            axis_place_odds_high: r.axis_place_odds_high,
            odds_missing: r.odds_missing,
            slip,
            flip: LiveFlip::from(r.flip),
        })
    }
}

impl From<LiveFlipView> for LiveFlip {
    fn from(f: LiveFlipView) -> Self {
        Self {
            axis_changed: f.axis_changed,
            prev_axis: f.prev_axis,
            ev_reversed: f.ev_reversed,
            prev_verdict: f.prev_verdict,
            prev_roi: f.prev_roi,
        }
    }
}
