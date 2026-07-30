pub mod card;
pub mod course;
pub mod entry;
pub mod horse;
pub mod horse_history;
pub mod jockey;
pub mod live;
pub mod maintenance;
pub mod odds;
pub mod pdf;
pub mod prediction;
pub mod race;
pub mod results;
pub mod settle;
pub mod trainer;

/// 非 PDF ユースケース（race/predict/board/live/stats 等）の facade（#453）。
/// PDF 取得・解析は [`pdf::PdfInteractor`] に分離済みで、ここは Repository のみを持つ
/// （かつての `Interactor<R, P: PdfParser, F: PdfFetcher>` から P/F ジェネリクスと Noop スタブを解消）。
pub struct Interactor<R> {
    pub repository: R,
}

impl<R> Interactor<R> {
    pub fn new(repository: R) -> Self {
        Self { repository }
    }
}
