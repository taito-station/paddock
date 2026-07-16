use paddock_domain::Race;

use crate::error::{Error, Result};

pub trait PdfParser: Send + Sync {
    fn parse(&self, bytes: &[u8]) -> Result<Vec<Race>>;
}

/// PDF を扱わない bin（predict / predict-watch / odds-collect / analyze / api-server）向けの
/// no-op スタブ（#410）。`Interactor<R, P: PdfParser, F: PdfFetcher>` が P/F を常時ジェネリクスで
/// 要求するため注入するが、これらの bin は PDF 系ユースケース（`ingest_pdf` / `fetch_meeting`）を
/// 呼ばないため `parse` は決して実行されない。誤って呼ばれた場合はサイレントな空成功にせず
/// `InvalidArgument` で明示的に失敗させる（従来 5 apps が各自定義していた `UnusedParser` を集約）。
pub struct NoopParser;

impl PdfParser for NoopParser {
    fn parse(&self, _bytes: &[u8]) -> Result<Vec<Race>> {
        Err(Error::InvalidArgument(
            "this binary does not parse PDFs".into(),
        ))
    }
}
