//! netkeiba スクレイパ（出馬表 → horse_id、馬個別成績 → 近走）。
//!
//! 当日出走馬の多くは自 DB に履歴が無く predict の確率が薄まる。netkeiba の馬 ID 単位
//! ページ `db.netkeiba.com/horse/result/<id>/` は全成績を静的 HTML で返すため、これを
//! 近走の供給源にする（`shutuba_past` は JS 描画で全頭取れず割当もズレる）。
//!
//! ネットワーク I/O（[`scraper::UreqNetkeibaScraper`]）と純粋なパース（[`parse`]）を分離し、
//! パースは fixture で網羅テストする。

pub mod error;
pub mod parse;
pub mod scraper;

pub use error::{Error, Result};
pub use scraper::UreqNetkeibaScraper;
