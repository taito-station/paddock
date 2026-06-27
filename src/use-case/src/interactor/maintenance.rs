use chrono::NaiveDate;

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::OddsRepository;

impl<R: OddsRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// `race_odds_snapshots`（append-only 履歴, #232）のうち `fetched_at` の日付が `before`
    /// より前の行をパージする（retention, #234）。`dry_run = true` のときは削除せず該当行数を
    /// 返す。返り値はどちらの場合も「対象（削除した or 削除予定の）行数」。
    ///
    /// 最新キャッシュ `race_odds` は対象外（snapshots 専用）。cutoff の決定（実行日 − 保持月数）は
    /// 呼び出し側（CLI）が UTC で行い、ここには確定済みの `before` 日付を渡す。
    pub async fn purge_old_odds_snapshots(&self, before: NaiveDate, dry_run: bool) -> Result<u64> {
        if dry_run {
            let count = self
                .repository
                .count_race_odds_snapshots_before(before)
                .await?;
            tracing::info!(
                %before,
                count,
                "race_odds_snapshots パージ dry-run（削除予定行数）"
            );
            Ok(count)
        } else {
            let deleted = self.repository.purge_race_odds_snapshots(before).await?;
            tracing::info!(%before, deleted, "race_odds_snapshots をパージした");
            Ok(deleted)
        }
    }
}
