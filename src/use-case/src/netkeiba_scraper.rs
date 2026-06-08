use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseId, HorseName, HorseNum, JockeyName, ResultStatus, Surface,
    TimeSeconds, TrackCondition, Venue,
};

use crate::error::Result;

/// 出馬表 1 頭分の参照情報。近走取得のキー `horse_id` を馬番・馬名に紐付ける。
#[derive(Debug, Clone, PartialEq)]
pub struct RunnerRef {
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    pub horse_id: HorseId,
}

/// netkeiba の馬個別成績ページ 1 行 = その馬の過去 1 走。
///
/// JRA 平地レースに正規化済み（障害・地方・海外は parse 層でスキップ）。`netkeiba_race_id`
/// は同一過去レースを走った複数馬を 1 レースへ集約するキーで、合成 race_id `nk-<id>` の元になる。
/// venue / round / day / race_num はこの 12 桁 ID から導出する。
#[derive(Debug, Clone, PartialEq)]
pub struct HorsePastRun {
    pub netkeiba_race_id: String,
    pub date: NaiveDate,
    pub venue: Venue,
    pub round: u32,
    pub day: u32,
    pub race_num: u32,
    pub surface: Surface,
    pub distance: u32,
    pub track_condition: Option<TrackCondition>,
    pub finishing_position: Option<FinishingPosition>,
    pub status: ResultStatus,
    pub gate_num: GateNum,
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    pub jockey: Option<JockeyName>,
    pub time_seconds: Option<TimeSeconds>,
    pub margin: Option<String>,
    pub odds: Option<f64>,
    pub horse_weight: Option<u32>,
    pub weight_change: Option<i32>,
    pub weight_carried: Option<f64>,
    pub popularity: Option<u32>,
}

/// 出馬表 1 頭分の登録情報（枠・馬番・馬名・騎手）。当日の `RaceCard` を組むための最小集合。
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedEntry {
    pub gate_num: GateNum,
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    pub jockey: Option<JockeyName>,
}

/// 出馬表ページ 1 件のパース結果。レースメタ（日付/場/距離 等）と全出走馬を持つ。
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedCard {
    pub date: NaiveDate,
    pub venue: Venue,
    pub round: u32,
    pub day: u32,
    pub race_num: u32,
    pub surface: Surface,
    pub distance: u32,
    pub entries: Vec<FetchedEntry>,
}

/// 単勝オッズ 1 頭分。レース前でオッズ未確定の馬はパース層で除外済み。
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedWinOdds {
    pub horse_num: HorseNum,
    pub odds: f64,
    pub popularity: Option<u32>,
}

/// Port for fetching netkeiba pages used to fill in same-day runners' recent form.
///
/// Implementations (Interface layer) own the HTTP fetch, EUC-JP decoding and HTML
/// parsing; the use-case layer depends only on this trait. Methods are synchronous
/// (ureq) and embed an inter-request delay out of courtesy to netkeiba. The
/// interactor is a single-shot CLI flow that calls these sequentially, so the
/// blocking I/O runs directly on the runtime thread (no `spawn_blocking`).
pub trait NetkeibaScraper: Send + Sync {
    /// 出馬表 (`race/shutuba.html`) から出走各馬の `horse_id` を馬番順に取得する。
    fn fetch_shutuba(&self, netkeiba_race_id: &str) -> Result<Vec<RunnerRef>>;

    /// 馬個別成績ページ (`horse/result/<id>/`) から JRA 平地の近走を取得する。
    fn fetch_horse_history(&self, horse_id: &HorseId) -> Result<Vec<HorsePastRun>>;

    /// 出馬表 (`race/shutuba.html`) から当日のレースカード（メタ + 全出走馬）を取得する。
    fn fetch_card(&self, netkeiba_race_id: &str) -> Result<FetchedCard>;

    /// 単勝オッズ API から各馬の単勝オッズ・人気を取得する。
    /// レース前でオッズ未確定の行はスキップされ、確定前は空 Vec を返し得る。
    fn fetch_win_odds(&self, netkeiba_race_id: &str) -> Result<Vec<FetchedWinOdds>>;
}
