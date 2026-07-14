use chrono::{NaiveDate, NaiveTime};

use crate::horse_result::{GateNum, HorseName, HorseNum, JockeyName, TrainerName};
use crate::race::{RaceClass, RaceId, Surface, Venue};

/// A single race's entry sheet (出馬表). Static pre-race information used as input for
/// pre-race tendency prediction; distinct from `Race` which carries day-of results.
#[derive(Debug, Clone)]
pub struct RaceCard {
    pub race_id: RaceId,
    /// 開催日。出馬表 PDF には日付テキストが無いため、取り込み元ファイル名の
    /// `YYYYMMDD` から導出してセットする（use-case 層の ingest で設定）。
    pub date: NaiveDate,
    /// 発走時刻（#235）。netkeiba 出馬表 RaceData01 の「HH:MM発走」から取得。
    /// 出馬表 PDF パーサは未対応のため PDF 経路は `None`。netkeiba 経路でも取得失敗時は `None`。
    pub post_time: Option<NaiveTime>,
    pub venue: Venue,
    pub round: u32,
    pub day: u32,
    pub race_num: u32,
    pub surface: Surface,
    pub distance: u32,
    /// レースの格付け／条件クラス（#345）。netkeiba 出馬表の `<title>` グレード表記と
    /// `RaceData02` 条件から導出。出馬表 PDF パーサは未対応のため PDF 経路は `None`。
    /// netkeiba 経路でも表記から判定できない場合は `None`。
    pub race_class: Option<RaceClass>,
    /// 表示用のレース名（#389）。netkeiba 出馬表の `h1.RaceName`（例「七夕賞」「響灘特別」
    /// 「3歳上1勝クラス」。グレード表記は含まない＝グレードは `race_class`）。出馬表 PDF パーサは
    /// 未対応のため PDF 経路は `None`。取得できない場合も `None`（best-effort・カード保存は止めない）。
    pub race_name: Option<String>,
    pub entries: Vec<HorseEntry>,
}

/// One horse entry in a race card. The minimum set required to look up tendencies
/// in the existing horse / jockey / course aggregations.
#[derive(Debug, Clone)]
pub struct HorseEntry {
    pub gate_num: GateNum,
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    pub jockey: Option<JockeyName>,
    /// 調教師（#74）。netkeiba 出馬表から取得。出馬表 PDF パーサは未対応のため、
    /// PDF 経路で取り込んだレースは `None`（確率推定で trainer 項なし）。
    pub trainer: Option<TrainerName>,
    /// 負担重量[kg]（#135）。netkeiba 出馬表から取得。出馬表 PDF パーサは未対応のため、
    /// PDF 経路で取り込んだレースは `None`（確率推定で斤量項なし）。
    pub weight_carried: Option<f64>,
}
