use std::collections::BTreeSet;
use std::future::Future;

use paddock_domain::{BetType, RaceId, RaceOdds};

use crate::error::Result;

/// 1 回のライブスクレイプの**観測結果**（#632）。
///
/// オッズ本体だけを返すと「その券種の行が無い」理由が呼び出し側から分からず、read-through は
/// 未発売の券種を永久に取り直し続ける（ADR 0086 が予告した副作用）。オッズと一緒に
/// 「netkeiba 上で未発売と確認できた券種」を返し、`OddsInteractor` が両者を区別できるようにする。
#[derive(Debug, Clone)]
pub struct ScrapedOdds {
    /// priced な（＝払戻倍率として採用できる）オッズ。
    pub odds: RaceOdds,
    /// **未発売と確認できた**組合せ券種（#632）。netkeiba が番兵しか返さなかった、または
    /// 取得に成功して 0 行だった券種が入る。**取得失敗は入らない**——失敗は「分からない」で
    /// あって「売っていない」ではなく、次回そのまま再取得させる必要があるため（#294 の自己修復）。
    ///
    /// 単勝・複勝は入れない。単複（type=1）は失敗を `Err` で伝播する経路（ベストエフォートでない）
    /// で、かつ番兵を持たない（ADR 0088）ため、「空＝未発売と確認」と読める状況が無い。
    pub unpriced: BTreeSet<BetType>,
}

/// Port for fetching live betting odds for a single race.
///
/// Implementations (Interface layer) own the HTTP fetch and response parsing;
/// the use-case layer only depends on this trait. Odds are scraped on demand
/// per race with no caching. The live implementation is `UreqNetkeibaScraper`
/// over the netkeiba odds API (UTF-8 JSON); the former JRA `accessO.html` path
/// was unverified (ADR 0001) and removed in #287.
///
/// メソッドは **async**（#458）。実装は同期ブロッキング I/O（`std::thread::sleep` + 同期 ureq
/// GET）を伴うため、async ハンドラ（api-server の actix worker）の経路では実装側で
/// `tokio::task::spawn_blocking` に逃がしてワーカースレッドを塞がないようにする。CLI 各 app は
/// 実質単一タスクなのでオフロードの有無に関わらず挙動は変わらない。戻り値の Future に `Send` を
/// 課すのは actix-web のマルチスレッドランタイム越しに握るため（conventions.md）。
pub trait OddsScraper: Send + Sync {
    /// 1 レース分の全券種を取得する。戻り値はオッズ本体＋未発売と確認できた券種
    /// （[`ScrapedOdds`]・#632）。
    fn scrape(&self, race_id: &RaceId) -> impl Future<Output = Result<ScrapedOdds>> + Send;

    /// 単勝・複勝**のみ**を取得する軽量経路（オッズ時系列コレクタ用・#odds-collect）。
    ///
    /// コレクタは全レースを終日・高頻度でスナップするため、組合せ 5 券種まで取る `scrape`
    /// （1 レース 6 GET）は重い。単勝中心の movement 収集では win/place（type=1・1 GET）で足りる。
    ///
    /// デフォルト実装は `scrape` の結果から win/place だけを残す（正しいが軽量でない）。
    /// ネットワーク実装（`UreqNetkeibaScraper`）は type=1 の 1 GET だけを打つよう **override** する。
    ///
    /// 単複だけの観測は組合せ券種について何も言わないため、[`ScrapedOdds::unpriced`] は返さない
    /// （この経路で未発売マークを更新してはいけない・#632）。
    fn scrape_win_place(&self, race_id: &RaceId) -> impl Future<Output = Result<RaceOdds>> + Send {
        async move {
            let full = self.scrape(race_id).await?.odds;
            let mut wp = RaceOdds::empty(race_id.clone());
            wp.win = full.win;
            wp.place = full.place;
            Ok(wp)
        }
    }
}
