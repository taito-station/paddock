//! 確率推定で扱う値オブジェクト・データ構造（factor レート、馬の確率、前走、標準タイム等）。

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::horse_result::{HorseName, HorseNum, HorseResult};
use crate::race::Surface;

#[derive(Debug, Clone, Copy, Default)]
pub struct RateTriple {
    pub win: f64,
    pub place: f64,
    pub show: f64,
}

/// 1 つの factor のレート（win/place/show）と、その算出母数となった出走数（#75）。
/// `starts` はベイズ縮約（少データほど prior へ寄せる）で信頼度の重みに使う。
#[derive(Debug, Clone, Copy)]
pub struct FactorStat {
    pub rate: RateTriple,
    pub starts: u32,
}

#[derive(Debug, Clone)]
pub struct HorseFactors {
    /// コース（場×距離×馬場）の枠順別成績。当該コース×枠区分の出走実績が無い馬は `None`
    /// （項と重みを母数から除外、ADR 0007/0014 の欠落項扱い）。
    pub course_gate: Option<FactorStat>,
    /// 馬の芝ダ別成績。当該 surface での出走実績が無い馬（新馬等）は `None`（母数除外、#81）。
    pub horse_surface: Option<FactorStat>,
    /// 馬の距離帯別成績。当該距離帯での出走実績が無い馬（初距離等）は `None`（母数除外、#81）。
    pub horse_distance: Option<FactorStat>,
    /// 騎手の芝ダ別成績。騎手未登録、または当該 surface での騎乗実績が無い馬は `None`
    /// （母数除外、#81 で 0 埋めから統一）。
    pub jockey_surface: Option<FactorStat>,
    /// 調教師の芝ダ別成績（#74）。調教師が欠落、または当該 surface での実績が無い馬は `None`
    /// （項と重みを母数から除外、ADR 0007 の欠落項扱い）。netkeiba 出馬表からのみ取得するため、
    /// PDF 経路で取り込んだレースは常に `None`。
    pub trainer_surface: Option<FactorStat>,
    /// 馬場状態（良/稍重/重/不良）別の馬成績（#73）。対象レースの馬場状態が未確定、または
    /// 該当馬場での出走実績が無い馬は `None`（項と重みを母数から除外、ADR 0007 の欠落項扱い）。
    pub horse_track_condition: Option<FactorStat>,
    /// 前走フォーム [0,1]（0.5=中立）。前走が無い／有効な signal が無い馬は `None`。
    /// win/place/show に同値で寄与する（フォームは方向に依らず全体を底上げ／押し下げる）。
    pub recent_form: Option<f64>,
    /// 斤量（負担重量）のレース内相対シグナル [0,1]（0.5=中立, #135）。当該レースの field 平均斤量に
    /// 対する相対値で、斤量が取れない馬（PDF 出馬表等）・field 平均が出せないレースは `None`
    /// （項と重みを母数から除外）。win/place/show に同値で寄与する。
    pub weight_carried: Option<f64>,
    /// 騎手直近フォームシグナル [0,1]（0.5=中立, #221）。騎手の直近 N 走の人気乖離平均で算出する。
    /// 騎手未登録・近走なし・全走欠損は `None`（項と重みを母数から除外、ADR 0007 準拠）。
    pub jockey_recent_form: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct HorseProbability {
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    pub win_prob: f64,
    pub place_prob: f64,
    pub show_prob: f64,
}

/// 騎手直近フォーム特徴量（#221）の算出に使う 1 走分の情報。`find_jockey_recent_runs` の戻り要素。
/// `finishing_position` / `popularity` は PDF 未記録や中止等で `None` になることがある。
#[derive(Debug, Clone)]
pub struct JockeyFormRun {
    pub finishing_position: Option<u32>,
    pub popularity: Option<u32>,
}

/// 馬の過去 1 走を、その走の (surface, distance) と開催日付きで返す（#31/#76）。前走タイムを
/// 標準タイムと突き合わせて相対速度に変換するため、成績本体 `result` に加えて当該レースの
/// surface/distance を運ぶ。`find_recent_runs` の戻り要素。
#[derive(Debug, Clone)]
pub struct RecentRun {
    pub date: NaiveDate,
    pub surface: Surface,
    pub distance: u32,
    pub result: HorseResult,
}

/// コーパス由来の標準タイム表（surface×distance 別の代表タイム[秒], #76）。前走タイムを
/// 「基準タイムに対する相対速度」へ変換する分母に使う。`date < before` で集計され（as-of で
/// リーク防止）、標本数が閾値未満の薄いバケツは含めない。該当 (surface,distance) が無ければ
/// `get` が `None` を返し、タイム sub-signal は母数から落ちる（欠落フォールバック）。
#[derive(Debug, Clone, Default)]
pub struct StandardTimes {
    by_course: HashMap<(Surface, u32), f64>,
}

impl StandardTimes {
    /// (surface, distance) → 標準タイム[秒] の表から構築する。
    pub fn new(by_course: HashMap<(Surface, u32), f64>) -> Self {
        Self { by_course }
    }

    /// 指定 (surface, distance) の標準タイム[秒]。未整備なら `None`。
    pub fn get(&self, surface: Surface, distance: u32) -> Option<f64> {
        self.by_course.get(&(surface, distance)).copied()
    }
}

/// 日付付きの 1 日分（または同一日複数走）の成績カウント（リーセンシー重み付け用, #75 Phase B）。
#[derive(Debug, Clone, Copy)]
pub struct DatedCounts {
    pub date: NaiveDate,
    pub starts: u32,
    pub wins: u32,
    pub places: u32,
    pub shows: u32,
}
