use chrono::NaiveDate;
use paddock_domain::RaceCard;

use crate::error::Result;

pub trait EntryParser: Send + Sync {
    /// 出馬表 PDF をパースする。`date` は取り込み元ファイル名から導出した開催日で、
    /// PDF 本文に日付が無いため呼び出し側が与える（各 `RaceCard` にセットされる）。
    fn parse(&self, bytes: &[u8], date: NaiveDate) -> Result<Vec<RaceCard>>;
}
