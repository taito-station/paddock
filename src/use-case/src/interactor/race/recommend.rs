use paddock_domain::{Portfolio, PortfolioConfig, RaceId, TrackCondition, build_portfolio};

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{OddsRepository, RaceCardRepository, StatsRepository};

impl<R: StatsRepository + RaceCardRepository + OddsRepository, P: PdfParser, F: PdfFetcher>
    Interactor<R, P, F>
{
    /// 予算内・軸流しポートフォリオ（馬連＋ワイド＋三連複）の買い目推奨を返す（#122 と同経路）。
    ///
    /// 確率は [`Self::predict_race`] と同じ推定（`blend_alpha` / `track_condition` も同義）。
    /// オッズは **保存済み（#51, `find_race_odds(.., None)` の最新スナップショット）** を参照し、
    /// ライブ取得はしない（鮮度更新は session の odds:refresh の責務）。保存オッズが無ければ
    /// `Ok(None)` を返す（呼び出し側は「最新取得」を促す）。出馬表が無ければ `predict_race` が
    /// `Error::NotFound` を返す。
    pub async fn recommend_bets(
        &self,
        race_id: &RaceId,
        budget: u64,
        blend_alpha: Option<f64>,
        track_condition: Option<TrackCondition>,
    ) -> Result<Option<Portfolio>> {
        let probs = self
            .predict_race(race_id, blend_alpha, track_condition)
            .await?;
        let Some(odds) = self.repository.find_race_odds(race_id, None).await? else {
            return Ok(None);
        };
        Ok(Some(build_portfolio(
            &probs,
            &odds,
            budget,
            &PortfolioConfig::default(),
        )))
    }
}
