use core::future::Future;
use std::collections::HashMap;

use chrono::{NaiveDate, NaiveTime};
use paddock_domain::{Race, RaceCard, RaceClass, RaceId};

use crate::error::Result;

/// レース本体（確定成績）の保存と存在判定・件数・日付検索。
pub trait RaceRepository: Send + Sync {
    fn save_race(&self, race: &Race) -> impl Future<Output = Result<()>> + Send;

    fn count_races(&self) -> impl Future<Output = Result<u64>> + Send;

    fn race_exists(&self, race_id: &RaceId) -> impl Future<Output = Result<bool>> + Send;

    /// 指定日に開催されるレース一覧を race_num 昇順で返す。
    /// 予想用途のため `results` は読み込まず空 Vec で返す。
    fn find_races_by_date(&self, date: NaiveDate)
    -> impl Future<Output = Result<Vec<Race>>> + Send;
}

/// 出馬表（race card）の保存・取得。
pub trait RaceCardRepository: Send + Sync {
    fn save_race_card(&self, card: &RaceCard) -> impl Future<Output = Result<()>> + Send;

    fn find_race_card(
        &self,
        race_id: &RaceId,
    ) -> impl Future<Output = Result<Option<RaceCard>>> + Send;

    /// 指定日の全レースの発走時刻を `race_id → post_time` で返す（`race_cards` 由来）。
    /// post_time 未保存のレースはマップに含まれない（#391）。
    fn find_post_times_by_date(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<HashMap<RaceId, NaiveTime>>> + Send;

    /// 指定日の全レースの表示用レース名を `race_id → race_name` で返す（`race_cards` 由来）。
    /// race_name 未保存のレースはマップに含まれない（#389）。
    fn find_race_names_by_date(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<HashMap<RaceId, String>>> + Send;

    /// 指定日の全レースのレースクラスを `race_id → race_class` で返す（`race_cards` 由来）。
    /// race_class 未保存のレースはマップに含まれない（#459・監視ループの G1 裏検出用一括取得）。
    fn find_race_classes_by_date(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<HashMap<RaceId, RaceClass>>> + Send;
}

/// レース結果の上位着順 1 行（read 用・#381）。ライブ一覧の着順表示に使う。
#[derive(Debug, Clone)]
pub struct FinishEntry {
    pub position: u32,
    pub horse_num: u32,
    pub horse_name: String,
}

/// レース結果（`results` の着順）の read＋同日 upsert（#381）。「着順が入っているか（結果確定）」
/// 「上位着順」「馬番→着順」を read し、同日取り込みで着順を upsert する。
pub trait RaceResultRepository: Send + Sync {
    /// 同日取り込み: `races` 行を出馬表メタから upsert（FK 担保）し、着順を `results` へ upsert する。
    /// 値カラムは COALESCE で既存値を温存し、`races` の track_condition/weather や他馬の既存着順を
    /// 破壊しない（`save_race` の無条件上書き・DELETE と異なる）。upsert した着順行数を返す。
    fn upsert_results(
        &self,
        card: &RaceCard,
        rows: &[crate::netkeiba_scraper::ResultRow],
    ) -> impl Future<Output = Result<u64>> + Send;

    /// 指定日の各レースの結果確定フラグ（`results` に `finishing_position IS NOT NULL` 行が 1 件以上）。
    /// 確定レースのみを `race_id → true` で返す（未確定はマップに含まれず、呼び出し側は false 既定）。
    fn find_result_confirmed_by_date(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<HashMap<RaceId, bool>>> + Send;

    /// 指定日の各レースの上位着順（`finishing_position <= 3`。3 着同着で 4 件以上返りうる＝件数可変）。
    /// 着順昇順。確定レースのみマップに含まれる。
    fn find_top_finishes_by_date(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<HashMap<RaceId, Vec<FinishEntry>>>> + Send;

    /// 指定レースの `馬番 → 着順`（board 用）。着順が入っている馬のみ含む。
    fn find_finishing_positions(
        &self,
        race_id: &RaceId,
    ) -> impl Future<Output = Result<HashMap<u32, u32>>> + Send;
}
