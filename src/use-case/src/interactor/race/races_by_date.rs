use std::collections::HashMap;

use chrono::{NaiveDate, NaiveTime};
use paddock_domain::{Race, RaceId};

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{FinishEntry, RaceCardRepository, RaceRepository, RaceResultRepository};

impl<R: RaceRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 指定日のレース一覧を race_num 昇順で取得する。
    pub async fn races_by_date(&self, date: NaiveDate) -> Result<Vec<Race>> {
        self.repository.find_races_by_date(date).await
    }
}

impl<R: RaceCardRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 指定日の全レースの発走時刻（`race_id → post_time`、race_cards 由来）を返す（#391）。
    /// レース一覧 API が watch 判定記録に依存せず発走時刻・状態を出すために使う。
    pub async fn post_times_by_date(&self, date: NaiveDate) -> Result<HashMap<RaceId, NaiveTime>> {
        self.repository.find_post_times_by_date(date).await
    }

    /// 指定日の全レースの表示用レース名（`race_id → race_name`、race_cards 由来）を返す（#389）。
    /// レース一覧 API が重賞・特別戦名をヘッダに出すために使う。
    pub async fn race_names_by_date(&self, date: NaiveDate) -> Result<HashMap<RaceId, String>> {
        self.repository.find_race_names_by_date(date).await
    }
}

impl<R: RaceResultRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 指定日の各レースの結果確定フラグ（`race_id → true`。確定レースのみ）を返す（#381）。
    /// ライブ一覧・レース盤で「⚫終」を post_time 推定でなく着順確定で判定するために使う。
    pub async fn result_confirmed_by_date(&self, date: NaiveDate) -> Result<HashMap<RaceId, bool>> {
        self.repository.find_result_confirmed_by_date(date).await
    }

    /// 指定日の各レースの上位着順（`race_id → Vec<FinishEntry>`）を返す（#381）。
    pub async fn top_finishes_by_date(
        &self,
        date: NaiveDate,
    ) -> Result<HashMap<RaceId, Vec<FinishEntry>>> {
        self.repository.find_top_finishes_by_date(date).await
    }
}
