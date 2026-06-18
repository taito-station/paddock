use chrono::NaiveDate;
use paddock_domain::Race;

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::RaceRepository;

impl<R: RaceRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 指定日のレース一覧を race_num 昇順で取得する。
    pub async fn races_by_date(&self, date: NaiveDate) -> Result<Vec<Race>> {
        self.repository.find_races_by_date(date).await
    }
}
