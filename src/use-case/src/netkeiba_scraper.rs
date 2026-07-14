use chrono::{NaiveDate, NaiveTime};
use paddock_domain::{
    FinishingPosition, GateNum, HorseId, HorseName, HorseNum, JockeyName, OrderedPair,
    OrderedTriple, Pair, RaceClass, ResultStatus, Surface, TimeSeconds, TrackCondition,
    TrainerName, Triple, Venue,
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
    /// レース名の生テキスト（例「有馬記念(GI)」「3歳未勝利」「1勝クラス」, #329 Phase0）。
    /// クラス（新馬〜G1 の順序尺度）は domain 層でここから正規化する。取得できない行は `None`。
    pub race_name: Option<String>,
    /// コーナー通過順位の生テキスト（例「10-9-5-5」= 各コーナーでの位置, #329 Phase0）。
    /// 脚質（逃げ/先行/差し/追込）は domain 層でここから導出する。中止等で欠く行は `None`。
    pub corner_positions: Option<String>,
    /// 出走頭数（例 15, #329 Phase1）。脚質（先行度）でコーナー通過順位を相対化する分母。
    /// 取得できない行は `None`。
    pub field_size: Option<u32>,
}

/// netkeiba レース結果ページ (`race/result.html`) 1 頭分の確定成績。
///
/// `results` テーブルの既存行（PDF 由来）を netkeiba 由来の clean な値で**更新**するための入力型。
/// jockey/trainer は netkeiba の略名表記で、出馬表(entry)経路と同一表記のため predict の join が
/// 噛み合う（PDF フルネーム/馬主混入の不整合を解消）。`horse_num` を更新キーとする。
#[derive(Debug, Clone, PartialEq)]
pub struct ResultRow {
    pub horse_num: HorseNum,
    pub finishing_position: Option<FinishingPosition>,
    pub status: ResultStatus,
    pub jockey: Option<JockeyName>,
    pub trainer: Option<TrainerName>,
    pub time_seconds: Option<TimeSeconds>,
    pub odds: Option<f64>,
    pub horse_weight: Option<u32>,
    pub weight_change: Option<i32>,
    pub weight_carried: Option<f64>,
    pub popularity: Option<u32>,
}

/// 出馬表 1 頭分の登録情報（枠・馬番・馬名・騎手・調教師）。当日の `RaceCard` を組むための最小集合。
/// `horse_id` は近走取り込み（#103）の再利用キー。出馬表保存の必須項目ではないため `Option`
/// とし、抽出できない馬がいても card 保存からは落とさない（近走取り込みの対象外になるだけ）。
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedEntry {
    pub gate_num: GateNum,
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    pub horse_id: Option<HorseId>,
    pub jockey: Option<JockeyName>,
    pub trainer: Option<TrainerName>,
    /// 負担重量[kg]（#135）。出馬表の斤量列から取得。欠損・パース不能は `None`。
    pub weight_carried: Option<f64>,
}

/// 出馬表ページ 1 件のパース結果。レースメタ（日付/場/距離 等）と全出走馬を持つ。
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedCard {
    pub date: NaiveDate,
    /// 発走時刻（#235）。RaceData01「HH:MM発走」から取得。取得失敗時は `None`（best-effort）。
    pub post_time: Option<NaiveTime>,
    pub venue: Venue,
    pub round: u32,
    pub day: u32,
    pub race_num: u32,
    pub surface: Surface,
    pub distance: u32,
    /// レースの格付け／条件クラス（#345）。`<title>` のグレード表記と `RaceData02` の
    /// 条件から判定。判定できなければ `None`。
    pub race_class: Option<RaceClass>,
    /// 表示用のレース名（#389）。`h1.RaceName`（グレード表記は含まない）。取得できなければ `None`。
    pub race_name: Option<String>,
    pub entries: Vec<FetchedEntry>,
}

/// 単勝オッズ 1 頭分。レース前でオッズ未確定の馬はパース層で除外済み。
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedWinOdds {
    pub horse_num: HorseNum,
    pub odds: f64,
    pub popularity: Option<u32>,
}

/// 複勝オッズ 1 頭分。netkeiba は複勝を下限〜上限の幅で公表するため両端を持つ。
/// レース前でオッズ未確定の馬はパース層で除外済み。
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedPlaceOdds {
    pub horse_num: HorseNum,
    pub odds_low: f64,
    pub odds_high: f64,
    pub popularity: Option<u32>,
}

/// 単勝・複勝オッズをまとめた取得結果。
///
/// netkeiba のオッズ API は 1 レスポンスに単勝(`data.odds["1"]`)と複勝(`data.odds["2"]`)を
/// 同梱するため、1 回の取得で両方を得る。確定前はそれぞれ空になり得る。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FetchedOdds {
    pub win: Vec<FetchedWinOdds>,
    pub place: Vec<FetchedPlaceOdds>,
}

/// 組合せ券種オッズ 1 点。netkeiba のオッズ API（type=4/6/7/8）由来で、組合せ（ドメインの
/// `Pair`/`OrderedPair`/`Triple`/`OrderedTriple`）に確定オッズと人気を紐付ける。レース前で
/// 未確定（`---.-`）の行はパース層で除外済み。
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedComboOdds<K> {
    pub combination: K,
    pub odds: f64,
    /// API は組合せ券種にも人気を返すため取り込むが、現状の永続化（`OddsRow`）は組合せ券種の
    /// 人気を保存しないため後段では未使用。将来 race_odds に人気を残す際に使えるよう保持する。
    pub popularity: Option<u32>,
}

/// ワイドオッズ 1 点（#187）。netkeiba は複勝同様に下限〜上限の幅で公表するため両端を持つ。
/// 無順序ペア（`Pair`）に帯 odds と人気を紐付ける。レース前で未確定（`---.-`）の行は
/// パース層で除外済み。
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedWideOdds {
    pub combination: Pair,
    pub odds_low: f64,
    pub odds_high: f64,
    /// API は人気も返すため取り込むが、現状の永続化（`OddsRow::wide`）は組合せ券種の人気を
    /// 保存しないため後段では未使用（馬連等と同方針）。将来の利用に備えて保持する。
    pub popularity: Option<u32>,
}

/// 馬連・ワイド・馬単・三連複・三連単のオッズ取得結果（#102, #187）。各券種は独立に空に
/// なり得る（未公開・確定前）。netkeiba は券種ごとに別 API（type=4/5/6/7/8）なので個別取得して束ねる。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FetchedExoticOdds {
    /// 馬連（無順序ペア）
    pub quinella: Vec<FetchedComboOdds<Pair>>,
    /// ワイド（無順序ペア・帯 odds）
    pub wide: Vec<FetchedWideOdds>,
    /// 馬単（順序付きペア）
    pub exacta: Vec<FetchedComboOdds<OrderedPair>>,
    /// 三連複（無順序トリプル）
    pub trio: Vec<FetchedComboOdds<Triple>>,
    /// 三連単（順序付きトリプル）
    pub trifecta: Vec<FetchedComboOdds<OrderedTriple>>,
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

    /// 単勝・複勝オッズ API から各馬の単勝・複勝オッズと人気を取得する。
    /// レース前でオッズ未確定の行はスキップされ、確定前は空の `FetchedOdds` を返し得る。
    fn fetch_win_place_odds(&self, netkeiba_race_id: &str) -> Result<FetchedOdds>;

    /// 馬連・ワイド・馬単・三連複・三連単オッズ API（type=4/5/6/7/8）から組合せ券種オッズを取得する（#102, #187）。
    /// レース前で未確定の行はスキップされ、確定前は空になり得る。既定実装は空を返す
    /// （組合せ券種を取得しないスクレイパ実装・テスト用フェイクとの後方互換）。
    fn fetch_exotic_odds(&self, _netkeiba_race_id: &str) -> Result<FetchedExoticOdds> {
        Ok(FetchedExoticOdds::default())
    }
}
