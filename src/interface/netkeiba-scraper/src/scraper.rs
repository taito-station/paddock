//! netkeiba への同期 HTTP アクセス（`ureq`）と charset 対応デコード。
//!
//! 純粋なパースは [`crate::parse`] に分離してあり、ここはネットワーク I/O のみ。
//! netkeiba 配慮のためリクエスト間に固定ウェイトを挟む。
//! 本文エンコーディングはホストで異なる（race=UTF-8 / db=EUC-JP）ため、
//! [`fetch_decoded`] が `Content-Type` の charset に従ってデコードする。

use std::io::Read;
use std::time::Duration;

use paddock_domain::HorseId;
use paddock_use_case::Result as UcResult;
use paddock_use_case::netkeiba_scraper::{
    FetchedCard, FetchedExoticOdds, FetchedOdds, HorsePastRun, NetkeibaScraper, RunnerRef,
};

use crate::error::{Error, Result};
use crate::parse;

const SHUTUBA_URL: &str = "https://race.netkeiba.com/race/shutuba.html";
const RACE_RESULT_URL: &str = "https://race.netkeiba.com/race/result.html";
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
        // ureq2 の単一 timeout_read を ureq3 ではレスポンス受信／ボディ受信の
        // 2 フェーズに分割。各フェーズに独立して READ_TIMEOUT を適用する
        // （ハング検知が目的で総予算の厳密一致は不要なため、各 30s で十分）。
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(READ_TIMEOUT))
            .timeout_recv_body(Some(READ_TIMEOUT))
            .build()
            .into();
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

    /// レース結果ページ (`race/result.html`) から確定成績（着順・騎手略名・調教師略名等）を取得する。
    /// 既存 `results` を netkeiba 由来の clean な値で更新する用途（`fetch-results` アプリ）。
    /// `NetkeibaScraper` トレイトには載せず、結果再取込フロー専用の inherent メソッドとする。
    pub fn fetch_race_result(
        &self,
        netkeiba_race_id: &str,
    ) -> UcResult<Vec<paddock_use_case::netkeiba_scraper::ResultRow>> {
        std::thread::sleep(self.delay);
        let url = format!("{RACE_RESULT_URL}?race_id={netkeiba_race_id}");
        tracing::debug!(race_id = %netkeiba_race_id, "fetching netkeiba race result");
        let html = fetch_decoded(&self.agent, &url)?;
        Ok(parse::parse_race_result(&html, netkeiba_race_id)?)
    }

    /// レース結果ページ (`race/result.html`) から確定払戻（単勝〜三連単）を取得する（#40）。
    /// `fetch_race_result` と同じ URL・charset 対応デコードを使い、payout ブロックをパースする。
    /// 未確定（払戻ブロック無し）は空の `RacePayouts` を返す。
    pub fn fetch_race_payouts(
        &self,
        netkeiba_race_id: &str,
    ) -> UcResult<paddock_domain::RacePayouts> {
        std::thread::sleep(self.delay);
        let url = format!("{RACE_RESULT_URL}?race_id={netkeiba_race_id}");
        tracing::debug!(race_id = %netkeiba_race_id, "fetching netkeiba race payouts");
        let html = fetch_decoded(&self.agent, &url)?;
        let race_id = paddock_use_case::paddock_race_id_from_netkeiba(netkeiba_race_id)?;
        Ok(parse::parse_race_payouts(&html, race_id)?)
    }

    /// オッズ API を券種 `type` 指定で GET し、UTF-8 JSON を返す（#102）。
    /// 単勝・複勝(type=1) と組合せ券種(type=4/5/6/7/8) で URL 構成は共通。
    fn fetch_odds_json(&self, netkeiba_race_id: &str, odds_type: u8) -> Result<String> {
        std::thread::sleep(self.delay);
        let url =
            format!("{WIN_ODDS_URL}?race_id={netkeiba_race_id}&type={odds_type}&action=update");
        tracing::debug!(race_id = %netkeiba_race_id, odds_type, "fetching netkeiba odds");
        // オッズ API は UTF-8 JSON。EUC-JP デコードしない。
        fetch_utf8(&self.agent, &url)
    }

    /// 組合せ券種 1 種を取得・パースする。失敗（HTTP/想定外 status 等）は warn ログを残して
    /// 空 Vec に倒し、他券種の取得を継続させる（券種単位のベストエフォート、#102）。
    fn fetch_one_exotic<T>(
        &self,
        netkeiba_race_id: &str,
        odds_type: u8,
        parse: impl Fn(&str) -> Result<Vec<T>>,
    ) -> Vec<T> {
        match self
            .fetch_odds_json(netkeiba_race_id, odds_type)
            .and_then(|json| parse(&json))
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    race_id = %netkeiba_race_id,
                    odds_type,
                    error = %e,
                    "組合せ券種オッズの取得に失敗、当該券種をスキップして継続"
                );
                Vec::new()
            }
        }
    }
}

/// URL を GET し、レスポンスの `Content-Type` charset に従って本文をデコードして返す。
///
/// netkeiba はホストで本文エンコーディングが異なる: `race.netkeiba.com`（出馬表・結果）は
/// `charset=UTF-8` を明示し、`db.netkeiba.com`（馬個別成績）は charset を返さず本文は EUC-JP。
/// charset を尊重し、不明時は EUC-JP にフォールバックする（`scraper_util::decode_html`）。
/// EUC-JP 固定デコードは race.netkeiba.com の UTF-8 化で文字化けする回帰を起こしていた。
fn fetch_decoded(agent: &ureq::Agent, url: &str) -> Result<String> {
    let resp = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| Error::Fetch(format!("GET {url}: {e}")))?;
    // ureq は 4xx/5xx を Err(StatusCode) にするためここに来るのは 2xx/3xx のみ。
    // ボディ受信前に Content-Type の charset を控える（受信後は resp が消費される）。
    let charset = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .and_then(scraper_util::charset_from_content_type);
    let mut bytes = Vec::new();
    resp.into_body()
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| Error::Fetch(format!("read body {url}: {e}")))?;
    Ok(scraper_util::decode_html(&bytes, charset.as_deref(), url))
}

/// URL を GET し、レスポンスボディを UTF-8 として（lossy で）受け取る。
/// オッズ API は UTF-8 JSON を返すため、EUC-JP デコードする [`fetch_decoded`] とは分ける。
fn fetch_utf8(agent: &ureq::Agent, url: &str) -> Result<String> {
    let resp = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| Error::Fetch(format!("GET {url}: {e}")))?;
    let mut bytes = Vec::new();
    resp.into_body()
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| Error::Fetch(format!("read body {url}: {e}")))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

impl paddock_use_case::PayoutFetcher for UreqNetkeibaScraper {
    fn fetch_race_payouts(&self, netkeiba_race_id: &str) -> UcResult<paddock_domain::RacePayouts> {
        UreqNetkeibaScraper::fetch_race_payouts(self, netkeiba_race_id)
    }
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

    fn fetch_win_place_odds(&self, netkeiba_race_id: &str) -> UcResult<FetchedOdds> {
        // type=1 のレスポンスに単勝(odds["1"])と複勝(odds["2"])が同梱されるため 1 回の GET で両方取る。
        let json = self.fetch_odds_json(netkeiba_race_id, 1)?;
        Ok(parse::parse_win_place_odds(&json)?)
    }

    fn fetch_exotic_odds(&self, netkeiba_race_id: &str) -> UcResult<FetchedExoticOdds> {
        // 馬連・ワイド・馬単・三連複・三連単は券種ごとに別 API（type=4/5/6/7/8）。取得間に delay が
        // 挟まるため 1 レースあたり 5 回の待ちが加わる。**券種ごとにベストエフォート**: 1 本の API が
        // 失敗しても他券種や手前の取得分を巻き添えにせず、取れた券種だけ返す（#102 レビュー反映, #187）。
        Ok(FetchedExoticOdds {
            quinella: self.fetch_one_exotic(netkeiba_race_id, 4, parse::parse_quinella_odds),
            wide: self.fetch_one_exotic(netkeiba_race_id, 5, parse::parse_wide_odds),
            exacta: self.fetch_one_exotic(netkeiba_race_id, 6, parse::parse_exacta_odds),
            trio: self.fetch_one_exotic(netkeiba_race_id, 7, parse::parse_trio_odds),
            trifecta: self.fetch_one_exotic(netkeiba_race_id, 8, parse::parse_trifecta_odds),
        })
    }
}
