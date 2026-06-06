use chrono::NaiveDate;

use crate::dto::entry::ingest::IngestEntryResponse;
use crate::entry_parser::EntryParser;
use crate::error::Result;
use crate::interactor::entry::EntryInteractor;
use crate::pdf_fetcher::PdfFetcher;
use crate::repository::Repository;

impl<R: Repository, E: EntryParser, F: PdfFetcher> EntryInteractor<R, E, F> {
    pub async fn ingest_entry_pdf(&self, source: &str) -> Result<IngestEntryResponse> {
        // 出馬表 PDF 本文には日付が無いため、取り込み元ファイル名の先頭 `YYYYMMDD`
        // から開催日を導出して各 RaceCard に持たせる。
        let date = entry_date_from_source(source)?;
        let bytes = if source.starts_with("http://") || source.starts_with("https://") {
            self.fetcher.fetch(source)?
        } else {
            std::fs::read(source).map_err(|e| {
                crate::Error::InvalidArgument(format!("failed to read {source}: {e}"))
            })?
        };
        let cards = self.entry_parser.parse(&bytes, date)?;
        let mut cards_saved = 0;
        let mut entries_saved = 0;
        for card in &cards {
            // A degraded parse can yield a card whose header was read but whose rows were all
            // skipped. Persisting it would run `save_race_card`'s unconditional DELETE and wipe
            // a previously-good ingest of the same race while inserting nothing. Skip instead.
            if card.entries.is_empty() {
                tracing::warn!(
                    race_id = %card.race_id,
                    "race card parsed with no entries, skipping save"
                );
                continue;
            }
            self.repository.save_race_card(card).await?;
            cards_saved += 1;
            entries_saved += card.entries.len();
        }
        Ok(IngestEntryResponse {
            cards_saved,
            entries_saved,
        })
    }
}

/// 取り込み元（ローカルパス or URL）の **ファイル名先頭 8 桁** `YYYYMMDD` を開催日として解釈する。
///
/// 例: `pdfs/entries/inbox/20260419-03nakayama08.pdf` → `2026-04-19`。
/// 出馬表 PDF には日付テキストが無く、命名規約上ファイル名に開催日が含まれるため
/// （`project_entry_pdf_no_date` の方針）。8 桁の数字で始まらない場合はエラー。
fn entry_date_from_source(source: &str) -> Result<NaiveDate> {
    let file_name = source.rsplit(['/', '\\']).next().unwrap_or(source);
    let ymd: String = file_name.chars().take(8).collect();
    NaiveDate::parse_from_str(&ymd, "%Y%m%d").map_err(|_| {
        crate::Error::InvalidArgument(format!(
            "出馬表ファイル名の日付が不正です(YYYYMMDD想定): {source}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::entry_date_from_source;
    use chrono::NaiveDate;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn derives_date_from_local_path() {
        let date = entry_date_from_source("pdfs/entries/inbox/20260419-03nakayama08.pdf").unwrap();
        assert_eq!(date, ymd(2026, 4, 19));
    }

    #[test]
    fn derives_date_from_bare_filename() {
        assert_eq!(
            entry_date_from_source("20260601-05tokyo12.pdf").unwrap(),
            ymd(2026, 6, 1)
        );
    }

    #[test]
    fn derives_date_from_url() {
        assert_eq!(
            entry_date_from_source("https://example.com/entries/20261225-01nakayama01.pdf")
                .unwrap(),
            ymd(2026, 12, 25)
        );
    }

    #[test]
    fn errors_when_no_leading_date() {
        assert!(entry_date_from_source("nakayama08-entries.pdf").is_err());
    }

    #[test]
    fn errors_on_impossible_date() {
        // 8 桁あるが日付として不正（13 月）。
        assert!(entry_date_from_source("20261301-foo.pdf").is_err());
    }
}
