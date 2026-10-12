use core::future::Future;

use chrono::NaiveDate;

use crate::error::Result;

/// 朝の選定（軸・相手・混戦）。レースごとに 1 行（`morning_selections`, #767）。
///
/// predict-watch が、監視窓に入る前に保存された「朝」の単勝スナップショット（当日 JST 0 時以降で
/// 最初の単勝・#724 と同じ定義）から組んだ選定で、以降のスイープと盤の買い目の固定の元になる
/// （REQ-D01-003・REQ-D23-007）。#448 の盤の「朝↔現」（最初に全券種がそろった時点）とは別物。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MorningSelection {
    pub race_id: String,
    pub date: NaiveDate,
    /// 軸の馬番。
    pub axis: u32,
    /// 相手の馬番。昇順・軸を含まない。
    pub partners: Vec<u32>,
    /// 混戦のボックスの馬番（昇順・軸を含む）。**空 `Vec` は「混戦ではない」を意味する**
    /// （[`crate::repository::LiveEvPin::konsen_band`] と同じ規約）。
    pub konsen_band: Vec<u32>,
    /// 朝のスナップショットの取得時刻（`race_odds_snapshots.fetched_at` をそのまま写した UTC rfc3339）。
    pub odds_fetched_at: String,
    /// 選定を計算した時刻（UTC rfc3339）。
    pub computed_at: String,
}

/// 朝の選定（`morning_selections`）の保存・読み出し（#767）。
pub trait MorningSelectionRepository: Send + Sync {
    /// 指定開催日の朝の選定を全レース分返す。並びは `race_id` 昇順。無ければ空 `Vec`。
    fn find_morning_selections_by_date(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<Vec<MorningSelection>>> + Send;

    /// 1 レースの朝の選定を返す。無ければ `None`。
    fn find_morning_selection(
        &self,
        race_id: &str,
    ) -> impl Future<Output = Result<Option<MorningSelection>>> + Send;

    /// 朝の選定を保存する。**先勝ち**: 同じレースの行が既にあれば何もしない
    /// （上書きしない。再起動・`--once`・二重起動でも最初の選定を保つ）。
    /// 保存した行を固定に使うときは、この後に読み直して DB の値を使う。
    fn save_morning_selection(
        &self,
        selection: &MorningSelection,
    ) -> impl Future<Output = Result<()>> + Send;
}
