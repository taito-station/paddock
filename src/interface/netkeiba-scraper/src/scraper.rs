//! netkeiba への同期 HTTP アクセス（`ureq`）と EUC-JP デコード。
//!
//! 純粋なパースは [`crate::parse`] に分離してあり、ここはネットワーク I/O のみ。
//! netkeiba 配慮のためリクエスト間に固定ウェイトを挟む。

use std::io::Read;
use std::time::Duration;

use paddock_domain::HorseId;
use paddock_use_case::Result as UcResult;
use paddock_use_case::netkeiba_scraper::{
    FetchedCard, FetchedWinOdds, HorsePastRun, NetkeibaScraper, RunnerRef,
};

use crate::error::{Error, Result};
use crate::parse;

const SHUTUBA_URL: &str = "https://race.netkeiba.com/race/shutuba.html";
const HORSE_RESULT_URL: &str = "https://db.netkeiba.com/horse/result";
const WIN_ODDS_URL: &str = "https://race.netkeiba.com/api/api_get_jra_odds.html";
const DEFAULT_DELAY: Duration = Duration::from_millis(1000);
// ハングした接続で CLI が無限に止まらないよう接続/読取にタイムアウトを設ける。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
// netkeiba は素の ureq UA を弾くことがあるためブラウザ風 UA を送る。
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

/// netkeiba スクレイパ（`ureq` 同期）。馬個別成績の取得ごとに [`Self::delay`] だけ待つ。
pub struct UreqNetkeibaScraper {
    agent: ureq::Agent,
    delay: Duration,
}

impl Default for UreqNetkeibaScraper {
    fn default() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .build();
        Self {
            agent,
            delay: DEFAULT_DELAY,
        }
    }
}

impl UreqNetkeibaScraper {
    pub fn new() -> Self {
        Self::default()
    }

    /// リクエスト間ウェイトを指定して生成する（`--interval` 用）。
    pub fn with_delay(delay: Duration) -> Self {
        Self {
            delay,
            ..Self::default()
        }
    }
}

/// URL を GET し、EUC-JP のレスポンスボディを UTF-8 へデコードして返す。
fn fetch_decoded(agent: &ureq::Agent, url: &str) -> Result<String> {
    let resp = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| Error::Fetch(format!("GET {url}: {e}")))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| Error::Fetch(format!("read body {url}: {e}")))?;
    // ureq は 4xx/5xx を Err(Status) にするためここに来るのは 2xx/3xx のみ。
    // それでもメンテ画面など別エンコーディングが返ると文字化けで後段の table 不検出に
    // 化けて原因が見えにくいので、EUC-JP として解釈できないバイトがあれば警告する。
    let (decoded, _, had_errors) = encoding_rs::EUC_JP.decode(&bytes);
    if had_errors {
        tracing::warn!(url, "response was not valid EUC-JP; parsing may fail");
    }
    Ok(decoded.into_owned())
}

/// URL を GET し、レスポンスボディを UTF-8 として（lossy で）受け取る。
/// オッズ API は UTF-8 JSON を返すため、EUC-JP デコードする [`fetch_decoded`] とは分ける。
fn fetch_utf8(agent: &ureq::Agent, url: &str) -> Result<String> {
    let resp = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| Error::Fetch(format!("GET {url}: {e}")))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| Error::Fetch(format!("read body {url}: {e}")))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

impl NetkeibaScraper for UreqNetkeibaScraper {
    fn fetch_shutuba(&self, netkeiba_race_id: &str) -> UcResult<Vec<RunnerRef>> {
        std::thread::sleep(self.delay);
        let url = format!("{SHUTUBA_URL}?race_id={netkeiba_race_id}");
        tracing::debug!(race_id = %netkeiba_race_id, "fetching netkeiba shutuba");
        let html = fetch_decoded(&self.agent, &url)?;
        Ok(parse::parse_shutuba(&html)?)
    }

    fn fetch_horse_history(&self, horse_id: &HorseId) -> UcResult<Vec<HorsePastRun>> {
        std::thread::sleep(self.delay);
        let url = format!("{HORSE_RESULT_URL}/{}/", horse_id.value());
        tracing::debug!(horse_id = %horse_id, "fetching netkeiba horse history");
        let html = fetch_decoded(&self.agent, &url)?;
        Ok(parse::parse_horse_history(&html)?)
    }

    fn fetch_card(&self, netkeiba_race_id: &str) -> UcResult<FetchedCard> {
        std::thread::sleep(self.delay);
        let url = format!("{SHUTUBA_URL}?race_id={netkeiba_race_id}");
        tracing::debug!(race_id = %netkeiba_race_id, "fetching netkeiba card");
        let html = fetch_decoded(&self.agent, &url)?;
        Ok(parse::parse_card(&html, netkeiba_race_id)?)
    }

    fn fetch_win_odds(&self, netkeiba_race_id: &str) -> UcResult<Vec<FetchedWinOdds>> {
        std::thread::sleep(self.delay);
        let url = format!(
            "{WIN_ODDS_URL}?race_id={netkeiba_race_id}&type=1&action=update"
        );
        tracing::debug!(race_id = %netkeiba_race_id, "fetching netkeiba win odds");
        // オッズ API は UTF-8 JSON。EUC-JP デコードしない。
        let json = fetch_utf8(&self.agent, &url)?;
        Ok(parse::parse_win_odds(&json)?)
    }
}
