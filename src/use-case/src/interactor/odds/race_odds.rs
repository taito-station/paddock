use paddock_domain::{RaceId, RaceOdds};

use crate::error::Result;
use crate::interactor::odds::OddsInteractor;
use crate::odds_scraper::OddsScraper;

impl<O: OddsScraper> OddsInteractor<O> {
    /// race_id のオッズをライブスクレイプで取得する。
    ///
    /// 取得できれば `Some(odds)`、未取得なら `None` を返す。「未取得」は次の 2 つを束ねる:
    /// - スクレイプ失敗（サイト改変・開催日外・ネットワーク等）→ warn ログを出して `None`
    /// - 取得成功だが全馬券種が空（オッズ未公開）→ `None`
    ///
    /// いずれも予想フロー側ではスキップ扱いになり、1 レースの取得失敗でセッション全体を
    /// 止めない（`select_bets` を呼ばず安全に次レースへ進める設計、predict-session.md 参照）。
    pub async fn race_odds(&self, race_id: &RaceId) -> Result<Option<RaceOdds>> {
        match self.scraper.scrape(race_id) {
            Ok(odds) if odds.is_empty() => {
                // 取得は成功したが全馬券種が空（未公開）。スクレイプ失敗（warn）と
                // 区別できるよう debug で記録し、運用時に原因を切り分けられるようにする。
                tracing::debug!(race_id = %race_id, "オッズ取得成功だが全馬券種が空（未公開）、スキップ");
                Ok(None)
            }
            Ok(odds) => Ok(Some(odds)),
            Err(e) => {
                tracing::warn!(race_id = %race_id, error = %e, "オッズ取得に失敗、スキップ");
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use paddock_domain::{HorseNum, OddsValue, RaceId, RaceOdds};

    use crate::error::{Error, Result};
    use crate::interactor::odds::OddsInteractor;
    use crate::odds_scraper::OddsScraper;

    /// テスト用の OddsScraper。scrape の戻り値を差し替えて 3 分岐を網羅する。
    struct FakeScraper {
        result: fn(&RaceId) -> Result<RaceOdds>,
    }

    impl OddsScraper for FakeScraper {
        fn scrape(&self, race_id: &RaceId) -> Result<RaceOdds> {
            (self.result)(race_id)
        }
    }

    fn race_id() -> RaceId {
        RaceId::try_from("2026-3-nakayama-8-1R").unwrap()
    }

    fn odds_with_win(race_id: RaceId) -> RaceOdds {
        let mut odds = RaceOdds::empty(race_id);
        let mut win = HashMap::new();
        win.insert(
            HorseNum::try_from(1).unwrap(),
            OddsValue::try_from(3.5).unwrap(),
        );
        odds.win = win;
        odds
    }

    #[tokio::test]
    async fn returns_some_when_odds_present() {
        let interactor = OddsInteractor::new(FakeScraper {
            result: |rid| Ok(odds_with_win(rid.clone())),
        });
        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(got.is_some());
        assert!(!got.unwrap().is_empty());
    }

    #[tokio::test]
    async fn returns_none_when_odds_empty() {
        // 取得成功だが未公開（全馬券種が空）→ スキップ扱いの None
        let interactor = OddsInteractor::new(FakeScraper {
            result: |rid| Ok(RaceOdds::empty(rid.clone())),
        });
        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn returns_none_on_scrape_error() {
        // スクレイプ失敗はセッションを止めず None で安全にスキップ
        let interactor = OddsInteractor::new(FakeScraper {
            result: |_| Err(Error::Internal("navigation failed".into())),
        });
        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(got.is_none());
    }
}
