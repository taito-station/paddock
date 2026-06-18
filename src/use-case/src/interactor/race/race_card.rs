use paddock_domain::{RaceCard, RaceId};

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::RaceCardRepository;

impl<R: RaceCardRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 指定レースの出馬表（race card）を取得する。未保存なら `None`。
    /// REST API（#33）の出馬表エンドポイント用。`predict_race` が内部で使う
    /// `repository.find_race_card` を、依存方向を崩さず handler から呼べるよう
    /// use-case メソッドとして薄くラップする。
    pub async fn race_card(&self, race_id: &RaceId) -> Result<Option<RaceCard>> {
        self.repository.find_race_card(race_id).await
    }
}
