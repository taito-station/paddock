use std::collections::HashMap;

use chrono::{NaiveDate, NaiveTime};
use paddock_domain::Race;

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{RaceCardRepository, RaceRepository};

impl<R: RaceRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 指定日のレース一覧を race_num 昇順で取得する。
    pub async fn races_by_date(&self, date: NaiveDate) -> Result<Vec<Race>> {
        self.repository.find_races_by_date(date).await
    }
}

impl<R: RaceCardRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 指定日の全レースの発走時刻（`race_id → post_time`、race_cards 由来）を返す（#391）。
    /// レース一覧 API が watch 判定記録に依存せず発走時刻・状態を出すために使う。
    pub async fn post_times_by_date(&self, date: NaiveDate) -> Result<HashMap<String, NaiveTime>> {
        self.repository.find_post_times_by_date(date).await
    }
}
