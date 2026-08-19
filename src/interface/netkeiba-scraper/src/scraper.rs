//! netkeiba への同期 HTTP アクセス（`ureq`）と charset 対応デコード。
//!
//! 純粋なパースは [`crate::parse`] に分離してあり、ここはネットワーク I/O のみ。
//! netkeiba 配慮のためリクエスト間に固定ウェイトを挟む。
//! 本文エンコーディングはホストで異なる（race=UTF-8 / db=EUC-JP）ため、
//! [`fetch_decoded`] が `Content-Type` の charset に従ってデコードする。

use std::collections::BTreeSet;
use std::io::Read;
use std::time::Duration;

use paddock_domain::{BetType, HorseId, OddsValue, PlaceOdds, RaceId, RaceOdds};
use paddock_use_case::Result as UcResult;
use paddock_use_case::netkeiba_race_id_from_paddock;
use paddock_use_case::netkeiba_scraper::{
    FetchedCard, FetchedExoticOdds, FetchedOdds, HorsePastRun, NetkeibaScraper, RunnerRef,
};
use paddock_use_case::odds_scraper::{OddsScraper, ScrapedOdds};

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

    /// レース結果ページ (`race/result.html`) を **1 回だけ取得** し、着順と確定払戻を両方パースする（#381）。
    /// 着順・払戻は同一 HTML に載るため、`fetch_race_result` と `fetch_race_payouts` を個別に叩く
    /// 二重取得を避ける（同日取り込み `ResultsInteractor` 用）。未確定は着順 空 Vec・払戻 空 `RacePayouts`。
    pub fn fetch_race_result_page(
        &self,
        netkeiba_race_id: &str,
    ) -> UcResult<(
        Vec<paddock_use_case::netkeiba_scraper::ResultRow>,
        paddock_domain::RacePayouts,
    )> {
        std::thread::sleep(self.delay);
        let url = format!("{RACE_RESULT_URL}?race_id={netkeiba_race_id}");
        tracing::debug!(race_id = %netkeiba_race_id, "fetching netkeiba race result page");
        let html = fetch_decoded(&self.agent, &url)?;
        let race_id = paddock_use_case::paddock_race_id_from_netkeiba(netkeiba_race_id)?;
        let results = parse::parse_race_result(&html, netkeiba_race_id)?;
        let payouts = parse::parse_race_payouts(&html, race_id)?;
        Ok((results, payouts))
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
    ///
    /// **観測できたかどうかは `Option` で表す**（#632）: `Some(rows)` = 取得できた
    /// （0 行なら「発売されていない」と確認できた）／`None` = 取得に失敗して分からない。
    /// 従来は失敗も「発売されていない」もどちらも空 Vec になり区別できなかったため、
    /// read-through が未発売の券種を永久に取り直していた。空 Vec に倒す挙動自体
    /// （＝1 券種の失敗で他を巻き添えにしない）は呼び出し側で維持する。
    ///
    /// **`bool` を使わないのは極性を型で守るため**。実際 `bool` 版では「失敗したか」と
    /// 「成功したか」の取り違えが起き、**取得失敗だけが観測済みとして記録される**状態を作った
    /// （＝本 issue の修正が効かず、かつ一過性失敗が未発売として TTL のあいだ固定される）。
    fn fetch_one_exotic<T>(
        &self,
        netkeiba_race_id: &str,
        odds_type: u8,
        parse: impl Fn(&str) -> Result<Vec<T>>,
    ) -> Option<Vec<T>> {
        observed_or_skipped(
            self.fetch_odds_json(netkeiba_race_id, odds_type)
                .and_then(|json| parse(&json)),
            netkeiba_race_id,
            odds_type,
        )
    }
}

/// `url` を GET し、transient 失敗時は指数バックオフ（1s/2s）で再試行する（#288, ADR 0021 を
/// netkeiba へ展開）。リトライ回数・バックオフ・transient 判定は `scraper_util::call_with_retry`
/// に一元化した共通ポリシー（#460）を使う。接続リセット等の一時障害を握り潰さず自動回復させ、
/// 単複オッズの「try1 失敗 / try2 成功」を透過的に解消する。
///
/// netkeiba のオッズ API は未発売を HTTP ではなく 200+JSON status（`yoso` 等）で返すため、jra の
/// ような 403/404=absent 概念は無く 4xx は単純に非 transient（即返し）。ヘッダ取得（`.call()`）のみ
/// 再試行し、ボディ読取中の失敗は呼び出し側で一発 [`Error::Fetch`] とする（jra-fetcher と同方針）。
/// 全試行の失敗は [`Error::Fetch`] に写像する。
fn call_with_retry(agent: &ureq::Agent, url: &str) -> Result<ureq::http::Response<ureq::Body>> {
    scraper_util::call_with_retry(url, || {
        agent.get(url).header("User-Agent", USER_AGENT).call()
    })
    .map_err(|err| Error::Fetch(format!("GET {url}: {err}")))
}

/// URL を GET し、レスポンスの `Content-Type` charset に従って本文をデコードして返す。
///
/// netkeiba はホストで本文エンコーディングが異なる: `race.netkeiba.com`（出馬表・結果）は
/// `charset=UTF-8` を明示し、`db.netkeiba.com`（馬個別成績）は charset を返さず本文は EUC-JP。
/// charset を尊重し、不明時は EUC-JP にフォールバックする（`scraper_util::decode_html`）。
/// EUC-JP 固定デコードは race.netkeiba.com の UTF-8 化で文字化けする回帰を起こしていた。
fn fetch_decoded(agent: &ureq::Agent, url: &str) -> Result<String> {
    let resp = call_with_retry(agent, url)?;
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
    let resp = call_with_retry(agent, url)?;
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

impl paddock_use_case::ResultPageFetcher for UreqNetkeibaScraper {
    /// **#458**: 結果ページ取得は同期 sleep + ureq GET を含むため `spawn_blocking` に逃がす。
    /// `results:refresh`（web が 45 秒間隔で自動ポーリング）は未確定レース数ぶんの取得を直列で
    /// 回すため、actix worker 上で同期ブロッキングすると同 worker の盤 UI リクエストが無応答になる。
    /// tokio の blocking pool へオフロードして worker を解放する。CLI（fetch-results 等）は単一
    /// タスクのため挙動は変わらない。
    async fn fetch_race_result_page(
        &self,
        netkeiba_race_id: &str,
    ) -> UcResult<(
        Vec<paddock_use_case::netkeiba_scraper::ResultRow>,
        paddock_domain::RacePayouts,
    )> {
        let this = self.clone_for_blocking();
        let netkeiba_race_id = netkeiba_race_id.to_owned();
        run_blocking(move || UreqNetkeibaScraper::fetch_race_result_page(&this, &netkeiba_race_id))
            .await
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
        // 取得できた券種を `observed` に控え、「未発売と確認できた」と読み違えないようにする（#632）。
        // 券種と odds_type の対応を取り違えないよう、記録は各取得の**直後**に置く
        // （離れた対応表にすると入れ替わってもコンパイルが通ってしまう）。
        let mut observed = BTreeSet::new();
        let quinella = record_observed(
            self.fetch_one_exotic(netkeiba_race_id, 4, parse::parse_quinella_odds),
            BetType::Quinella,
            &mut observed,
        );
        let wide = record_observed(
            self.fetch_one_exotic(netkeiba_race_id, 5, parse::parse_wide_odds),
            BetType::Wide,
            &mut observed,
        );
        let exacta = record_observed(
            self.fetch_one_exotic(netkeiba_race_id, 6, parse::parse_exacta_odds),
            BetType::Exacta,
            &mut observed,
        );
        let trio = record_observed(
            self.fetch_one_exotic(netkeiba_race_id, 7, parse::parse_trio_odds),
            BetType::Trio,
            &mut observed,
        );
        let trifecta = record_observed(
            self.fetch_one_exotic(netkeiba_race_id, 8, parse::parse_trifecta_odds),
            BetType::Trifecta,
            &mut observed,
        );
        Ok(FetchedExoticOdds {
            quinella,
            wide,
            exacta,
            trio,
            trifecta,
            observed,
        })
    }
}

/// netkeiba の単複・組合せ券種オッズ DTO を 1 レース分の [`RaceOdds`] へ組み立てる純関数。
///
/// [`UreqNetkeibaScraper::scrape`] のネットワーク非依存な核（fetch と分離して単体テスト可能にする）。
/// DTO は生 f64 を持つため、ここでドメインの `OddsValue`/`PlaceOdds`（finite かつ `>= 1.0`、複勝は
/// low<=high）へ変換する。API は妥当値を返すが、変換に失敗する行（想定外の `< 1.0` 等）は
/// **その行だけ skip** し、レース全体を落とさない（取りこぼし耐性）。組合せ券種は DTO 段階で
/// 既にドメイン型キー（`Pair`/`OrderedPair`/`Triple`/`OrderedTriple`）を持つのでキー変換は不要。
///
/// 番兵判定は券種別（#630）なので、7 つの独立ループがそれぞれ自分の [`BetType`] を静的に渡す
/// （このループ構造が券種の正）。
///
/// **未発売の観測（#632）**: 組合せ 5 券種について「取得は成功したのに priced な行が 1 つも
/// 残らなかった」券種を [`ScrapedOdds::unpriced`] に載せる。判定を「番兵だった行があるか」では
/// なく「priced が 0 件か」に置くのは、実地のワイド未発売行が `["9999.9", "0.0", "--"]` の形で
/// 返り、相方の `0.0` が番兵ではなく**値域違反**として弾かれるため（番兵の有無だけを見ると
/// ワイドの未発売を取りこぼす）。JRA がそもそも売らない極小頭数レースの券種も同じ経路で
/// unpriced になり、ADR 0010 が「許容する」としていた毎回再スクレイプもここで閉じる。
/// 組合せ券種 1 種の取得結果を「観測できた（`Some`）/ できなかった（`None`）」へ写す純関数（#632）。
///
/// ネットワークに触らないので、`Ok` / `Err` の写し分けを単体テストで固定できる
/// （[`UreqNetkeibaScraper::fetch_one_exotic`] は HTTP を打つため直接は叩けない）。
/// 失敗は warn を残してから `None` にする——「取れなかった」を握り潰さないのは #287 以来の方針。
fn observed_or_skipped<T>(
    result: Result<Vec<T>>,
    netkeiba_race_id: &str,
    odds_type: u8,
) -> Option<Vec<T>> {
    match result {
        Ok(rows) => Some(rows),
        Err(e) => {
            tracing::warn!(
                race_id = %netkeiba_race_id,
                odds_type,
                error = %e,
                "組合せ券種オッズの取得に失敗、当該券種をスキップして継続"
            );
            None
        }
    }
}

/// 観測できた券種を `observed` に記録しつつ行を取り出す（#632）。
///
/// `None`（取得失敗）は**記録せず**空 Vec に倒す——1 券種の失敗で他券種を巻き添えにしない
/// （#102 のベストエフォート）一方、「未発売と確認できた」とは扱わないため次回そのまま
/// 取り直される（#294 の自己修復）。
fn record_observed<T>(
    rows: Option<Vec<T>>,
    bet_type: BetType,
    observed: &mut BTreeSet<BetType>,
) -> Vec<T> {
    match rows {
        Some(rows) => {
            observed.insert(bet_type);
            rows
        }
        None => Vec::new(),
    }
}

pub(crate) fn assemble_netkeiba(
    odds: &FetchedOdds,
    exotic: &FetchedExoticOdds,
    race_id: RaceId,
) -> ScrapedOdds {
    let mut out = RaceOdds::empty(race_id);
    for w in &odds.win {
        if let Ok(v) = OddsValue::try_from((BetType::Win, w.odds)) {
            out.win.insert(w.horse_num, v);
        }
    }
    for p in &odds.place {
        if let (Ok(low), Ok(high)) = (
            OddsValue::try_from((BetType::Place, p.odds_low)),
            OddsValue::try_from((BetType::Place, p.odds_high)),
        ) && let Ok(band) = PlaceOdds::try_from((low, high))
        {
            out.place.insert(p.horse_num, band);
        }
    }
    for q in &exotic.quinella {
        if let Ok(v) = OddsValue::try_from((BetType::Quinella, q.odds)) {
            out.quinella.insert(q.combination, v);
        }
    }
    for w in &exotic.wide {
        if let (Ok(low), Ok(high)) = (
            OddsValue::try_from((BetType::Wide, w.odds_low)),
            OddsValue::try_from((BetType::Wide, w.odds_high)),
        ) && let Ok(band) = PlaceOdds::try_from((low, high))
        {
            out.wide.insert(w.combination, band);
        }
    }
    for e in &exotic.exacta {
        if let Ok(v) = OddsValue::try_from((BetType::Exacta, e.odds)) {
            out.exacta.insert(e.combination, v);
        }
    }
    for t in &exotic.trio {
        if let Ok(v) = OddsValue::try_from((BetType::Trio, t.odds)) {
            out.trio.insert(t.combination, v);
        }
    }
    for t in &exotic.trifecta {
        if let Ok(v) = OddsValue::try_from((BetType::Trifecta, t.odds)) {
            out.trifecta.insert(t.combination, v);
        }
    }

    // 取得に成功したのに priced が 0 件だった組合せ券種を「未発売と確認できた」として載せる。
    // 観測できていない券種（取得失敗・そもそも取りに行っていない）は「分からない」なので
    // 載せない＝次回そのまま再取得させる。
    let mut unpriced = BTreeSet::new();
    for (bet_type, priced_count) in [
        (BetType::Quinella, out.quinella.len()),
        (BetType::Wide, out.wide.len()),
        (BetType::Exacta, out.exacta.len()),
        (BetType::Trio, out.trio.len()),
        (BetType::Trifecta, out.trifecta.len()),
    ] {
        if priced_count == 0 && exotic.observed.contains(&bet_type) {
            unpriced.insert(bet_type);
        }
    }

    ScrapedOdds {
        odds: out,
        unpriced,
    }
}

impl UreqNetkeibaScraper {
    /// spawn_blocking に move するための軽量クローン（#458）。`ureq::Agent` は内部 `Arc` 共有で
    /// clone が安価、`delay` は `Copy`。`'static` な自己コピーを作ってブロッキング処理へ渡す。
    fn clone_for_blocking(&self) -> Self {
        Self {
            agent: self.agent.clone(),
            delay: self.delay,
        }
    }

    /// blocking 版の full scrape（単複＋組合せ券種）。同期 sleep + ureq GET を含む。
    /// async な [`OddsScraper::scrape`] からは `spawn_blocking` 経由で呼ぶ（#458）。
    ///
    /// 失敗の扱いは券種の重要度で分ける:
    /// - **単複(type=1)は EV/ROI と市場単勝 α ブレンドの基礎**。取得失敗を握り潰して win 欠落の
    ///   部分オッズを返すと、read-through 経路では `persist_all` がそれを永続化し、以降 cache-hit で
    ///   不完全スナップショットが再利用されて誤判定を生む。よって失敗は `Err` を伝播し、`OddsInteractor`
    ///   側で当該レースを skip(`None`) させる（#287 の silent failure 撲滅の趣旨。旧 JRA 経路の
    ///   「token 解決失敗＝loud skip」と挙動を揃える）。
    /// - **組合せ券種は券種ごとにベストエフォート**（`fetch_one_exotic` が各 type の失敗を空 Vec に
    ///   畳むため `fetch_exotic_odds` は実質常に `Ok`）。1 券種の欠落で単複ベースの判定を巻き添えにしない。
    ///
    /// RaceId が JRA 形式でない（合成 `nk-` 等）場合は変換エラーをそのまま伝播する。
    fn scrape_blocking(&self, race_id: &RaceId) -> UcResult<ScrapedOdds> {
        let netkeiba_id = netkeiba_race_id_from_paddock(race_id)?;
        let odds = self.fetch_win_place_odds(&netkeiba_id)?;
        // 現状 `fetch_exotic_odds` は券種ごとに Err を空 Vec へ畳むため常に Ok だが、将来エラー伝播へ
        // 変わっても win ベース判定を巻き添えにしないよう、ここはベストエフォート（空で継続）に倒す。
        // その fallback（`default()`）は `observed` が空＝「何も観測していない」なので、
        // 「1 件も取れなかった」を「全券種が未発売と確認できた」と読み違えることはない（#632）。
        let exotic = self.fetch_exotic_odds(&netkeiba_id).unwrap_or_default();
        Ok(assemble_netkeiba(&odds, &exotic, race_id.clone()))
    }

    /// blocking 版の単複のみ（type=1・1 GET）取得。オッズ時系列コレクタが全レースを終日高頻度で
    /// スナップするため、組合せ券種を打たず netkeiba への負荷を最小化する。
    ///
    /// 組合せ券種は**一度も見ていない**ので、未発売の観測は返さない（#632）。`observed` が空の
    /// `FetchedExoticOdds::default()` を渡すことでそれが保証される（既定値が「何も観測していない」）。
    fn scrape_win_place_blocking(&self, race_id: &RaceId) -> UcResult<RaceOdds> {
        let netkeiba_id = netkeiba_race_id_from_paddock(race_id)?;
        let odds = self.fetch_win_place_odds(&netkeiba_id)?;
        let scraped = assemble_netkeiba(&odds, &FetchedExoticOdds::default(), race_id.clone());
        debug_assert!(scraped.unpriced.is_empty());
        Ok(scraped.odds)
    }
}

impl OddsScraper for UreqNetkeibaScraper {
    /// 内部 `RaceId` を netkeiba 12 桁へ変換し、単複・組合せ券種オッズ API（UTF-8 JSON）から
    /// ライブオッズを取得して [`RaceOdds`] を組み立てる。旧 JRA `accessO.html` cname 経路
    /// （ADR 0001 で未検証・実質機能せず #287 で撤去）を置き換え、fetch-card と同一の取得経路に統一する。
    ///
    /// **#458**: 同期 sleep + ureq GET は `spawn_blocking` に逃がす。actix worker（単一スレッド
    /// ランタイム）の経路で同期ブロッキングすると同 worker の全接続が止まるため、ブロッキング処理を
    /// tokio の blocking pool へオフロードして worker を解放する。CLI 各 app は単一タスクのため
    /// オフロードの有無で挙動は変わらない。失敗伝播・ベストエフォートの方針は
    /// [`Self::scrape_blocking`] を参照。
    async fn scrape(&self, race_id: &RaceId) -> UcResult<ScrapedOdds> {
        let this = self.clone_for_blocking();
        let race_id = race_id.clone();
        run_blocking(move || this.scrape_blocking(&race_id)).await
    }

    /// 単複のみ（type=1・1 GET）の軽量取得。オッズ時系列コレクタが全レースを終日高頻度で
    /// スナップするため、組合せ券種を打たず netkeiba への負荷を最小化する（trait 既定の
    /// 「full scrape して win/place を残す」を 1 GET へ override）。#458 で同期部を `spawn_blocking` 化。
    async fn scrape_win_place(&self, race_id: &RaceId) -> UcResult<RaceOdds> {
        let this = self.clone_for_blocking();
        let race_id = race_id.clone();
        run_blocking(move || this.scrape_win_place_blocking(&race_id)).await
    }
}

/// ブロッキング処理を tokio の blocking pool で実行し、結果を await 可能にする（#458）。
///
/// 呼び出しは常に tokio ランタイム内（api-server の `#[actix_web::main]` / CLI の `#[tokio::main]`）
/// で行われるため `spawn_blocking` は有効。join エラー（blocking タスクの panic 等）は
/// [`Error::Fetch`] に写像して呼び出し側へ伝える（use-case 側で当該レース skip に倒れる）。
async fn run_blocking<T, F>(f: F) -> UcResult<T>
where
    F: FnOnce() -> UcResult<T> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(res) => res,
        Err(join_err) => {
            Err(Error::Fetch(format!("blocking scrape task failed: {join_err}")).into())
        }
    }
}

#[cfg(test)]
mod tests {
    use paddock_domain::{HorseNum, OrderedPair, OrderedTriple, Pair, RaceId, Triple};
    use paddock_use_case::netkeiba_scraper::{
        FetchedComboOdds, FetchedPlaceOdds, FetchedWideOdds, FetchedWinOdds,
    };

    use super::*;

    fn h(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    /// 組合せ 5 券種すべてを「取得できた」とする観測（#632）。テストで
    /// `FetchedExoticOdds` を手組みするときの既定。
    fn all_exotic() -> BTreeSet<BetType> {
        BTreeSet::from([
            BetType::Quinella,
            BetType::Wide,
            BetType::Exacta,
            BetType::Trio,
            BetType::Trifecta,
        ])
    }

    #[test]
    fn assembles_all_bet_types_into_race_odds() {
        let race_id = RaceId::try_from("2026-1-hakodate-6-5R").unwrap();
        let odds = FetchedOdds {
            win: vec![FetchedWinOdds {
                horse_num: h(1),
                odds: 3.5,
                popularity: Some(2),
            }],
            place: vec![FetchedPlaceOdds {
                horse_num: h(1),
                odds_low: 1.5,
                odds_high: 2.0,
                popularity: Some(2),
            }],
        };
        let exotic = FetchedExoticOdds {
            quinella: vec![FetchedComboOdds {
                combination: Pair::try_from((h(1), h(2))).unwrap(),
                odds: 12.4,
                popularity: None,
            }],
            wide: vec![FetchedWideOdds {
                combination: Pair::try_from((h(1), h(2))).unwrap(),
                odds_low: 3.1,
                odds_high: 4.8,
                popularity: None,
            }],
            exacta: vec![FetchedComboOdds {
                combination: OrderedPair::try_from((h(2), h(1))).unwrap(),
                odds: 25.0,
                popularity: None,
            }],
            trio: vec![FetchedComboOdds {
                combination: Triple::try_from((h(1), h(2), h(3))).unwrap(),
                odds: 88.0,
                popularity: None,
            }],
            trifecta: vec![FetchedComboOdds {
                combination: OrderedTriple::try_from((h(3), h(1), h(2))).unwrap(),
                odds: 410.0,
                popularity: None,
            }],
            observed: all_exotic(),
        };

        let scraped = assemble_netkeiba(&odds, &exotic, race_id);
        let got = &scraped.odds;
        assert_eq!(got.win.len(), 1);
        assert!((got.win[&h(1)].value() - 3.5).abs() < 1e-9);
        let place = &got.place[&h(1)];
        assert!((place.low.value() - 1.5).abs() < 1e-9);
        assert!((place.high.value() - 2.0).abs() < 1e-9);
        assert_eq!(got.quinella.len(), 1);
        assert_eq!(got.wide.len(), 1);
        assert_eq!(got.exacta.len(), 1);
        assert_eq!(got.trio.len(), 1);
        assert_eq!(got.trifecta.len(), 1);
        assert!(!got.is_empty());
        // 全券種 priced なので未発売の観測は出ない。
        assert!(scraped.unpriced.is_empty());
    }

    #[test]
    fn skips_invalid_odds_rows_without_dropping_others() {
        // odds < 1.0 は OddsValue 変換に失敗する想定外行。その行だけ skip し、妥当行は残す。
        let race_id = RaceId::try_from("2026-1-hakodate-6-5R").unwrap();
        let odds = FetchedOdds {
            win: vec![
                FetchedWinOdds {
                    horse_num: h(1),
                    odds: 0.5, // 不正（< 1.0）→ skip
                    popularity: None,
                },
                FetchedWinOdds {
                    horse_num: h(2),
                    odds: 4.2, // 妥当 → 残す
                    popularity: None,
                },
            ],
            place: vec![],
        };
        let got = assemble_netkeiba(&odds, &FetchedExoticOdds::default(), race_id).odds;
        assert_eq!(got.win.len(), 1);
        assert!(got.win.contains_key(&h(2)));
        assert!(!got.win.contains_key(&h(1)));
    }

    #[test]
    fn skips_netkeiba_unpriced_sentinels() {
        // netkeiba は未発売の組み合わせに 99999.9 等の番兵を入れる。払戻倍率ではないので
        // ドメインに載せない（#621）。従来ワイドだけが落ちていたのは相方 odds_high=0.0 が
        // 下限違反だったからで、スカラー券種（馬連・三連複・三連単）は素通りしていた。
        let race_id = RaceId::try_from("2026-1-hakodate-6-5R").unwrap();
        let exotic = FetchedExoticOdds {
            quinella: vec![
                FetchedComboOdds {
                    combination: Pair::try_from((h(1), h(2))).unwrap(),
                    odds: 99_999.9, // 未発売 → skip
                    popularity: None,
                },
                FetchedComboOdds {
                    combination: Pair::try_from((h(3), h(4))).unwrap(),
                    odds: 12.4, // 妥当 → 残す
                    popularity: None,
                },
            ],
            trio: vec![
                FetchedComboOdds {
                    combination: Triple::try_from((h(3), h(7), h(15))).unwrap(),
                    odds: 99_999.9, // #621 の実害そのもの（EV=138.44 を作っていた脚）
                    popularity: None,
                },
                FetchedComboOdds {
                    combination: Triple::try_from((h(2), h(4), h(9))).unwrap(),
                    odds: 9_999.9, // 三連複の 9999.9 は正当な配当（#630）→ 残す
                    popularity: None,
                },
            ],
            wide: vec![
                FetchedWideOdds {
                    combination: Pair::try_from((h(1), h(3))).unwrap(),
                    odds_low: 9_999.9, // ワイドの番兵（#630。両端が番兵でも落ちる）→ skip
                    odds_high: 9_999.9,
                    popularity: None,
                },
                FetchedWideOdds {
                    combination: Pair::try_from((h(2), h(4))).unwrap(),
                    odds_low: 3.1, // 妥当 → 残す
                    odds_high: 4.9,
                    popularity: None,
                },
            ],
            trifecta: vec![
                FetchedComboOdds {
                    combination: OrderedTriple::try_from((h(3), h(1), h(2))).unwrap(),
                    odds: 999_999.9, // 三連単の番兵 → skip
                    popularity: None,
                },
                FetchedComboOdds {
                    combination: OrderedTriple::try_from((h(1), h(2), h(3))).unwrap(),
                    odds: 111_971.9, // 正当な高配当 → 残す（上限方式を採らない理由）
                    popularity: None,
                },
            ],
            observed: all_exotic(),
            ..FetchedExoticOdds::default()
        };

        let scraped = assemble_netkeiba(&FetchedOdds::default(), &exotic, race_id);
        // 馬単は行が 1 つも無く取得失敗でもない＝「未発売と確認できた」（#632）。
        // priced が残った 4 券種はマークしない。
        assert_eq!(scraped.unpriced, BTreeSet::from([BetType::Exacta]));
        let got = &scraped.odds;
        assert_eq!(got.quinella.len(), 1, "番兵の馬連だけ落ちる");
        assert!(
            got.quinella
                .contains_key(&Pair::try_from((h(3), h(4))).unwrap())
        );
        assert_eq!(
            got.trio.len(),
            1,
            "番兵の三連複だけ落ちる（9999.9 は正当・#630）"
        );
        assert!(
            got.trio
                .contains_key(&Triple::try_from((h(2), h(4), h(9))).unwrap())
        );
        assert_eq!(
            got.wide.len(),
            1,
            "ワイドの 9999.9 は番兵として落ちる（#630）"
        );
        assert!(
            got.wide
                .contains_key(&Pair::try_from((h(2), h(4))).unwrap())
        );
        assert_eq!(got.trifecta.len(), 1, "正当な高配当は残す");
        assert!(
            got.trifecta
                .contains_key(&OrderedTriple::try_from((h(1), h(2), h(3))).unwrap())
        );
    }

    #[test]
    fn empty_inputs_yield_empty_race_odds() {
        let race_id = RaceId::try_from("2026-1-hakodate-6-5R").unwrap();
        let got = assemble_netkeiba(
            &FetchedOdds::default(),
            &FetchedExoticOdds::default(),
            race_id,
        );
        assert!(got.odds.is_empty());
    }

    #[test]
    fn all_sentinel_bet_type_is_observed_as_unpriced() {
        // #632: 券種まるごと未発売（全行が番兵）は「未発売と確認できた」。これを記録しないと
        // race_odds に行が 1 つも入らず is_complete が永久 false になり、read-through が
        // 毎回 6 GET を打ち続ける。
        let race_id = RaceId::try_from("2026-1-hakodate-6-5R").unwrap();
        let exotic = FetchedExoticOdds {
            trio: vec![
                FetchedComboOdds {
                    combination: Triple::try_from((h(1), h(2), h(3))).unwrap(),
                    odds: 99_999.9,
                    popularity: None,
                },
                FetchedComboOdds {
                    combination: Triple::try_from((h(1), h(2), h(4))).unwrap(),
                    odds: 99_999.9,
                    popularity: None,
                },
            ],
            observed: all_exotic(),
            ..FetchedExoticOdds::default()
        };

        let scraped = assemble_netkeiba(&FetchedOdds::default(), &exotic, race_id);
        assert!(scraped.odds.trio.is_empty());
        assert!(scraped.unpriced.contains(&BetType::Trio));
    }

    #[test]
    fn real_wide_unpriced_shape_is_observed_as_unpriced() {
        // 実地のワイド未発売行は `["9999.9", "0.0", "--"]`（QA-odds-sentinel-621 Q2）。相方 0.0 は
        // 番兵ではなく**値域違反**なので、「全行が番兵か」で判定すると取りこぼす。判定を
        // 「priced が 0 件か」に置いている理由の回帰ガード。
        let race_id = RaceId::try_from("2026-1-hakodate-6-5R").unwrap();
        let exotic = FetchedExoticOdds {
            wide: vec![FetchedWideOdds {
                combination: Pair::try_from((h(1), h(2))).unwrap(),
                odds_low: 9_999.9,
                odds_high: 0.0,
                popularity: None,
            }],
            observed: all_exotic(),
            ..FetchedExoticOdds::default()
        };

        let scraped = assemble_netkeiba(&FetchedOdds::default(), &exotic, race_id);
        assert!(scraped.odds.wide.is_empty());
        assert!(scraped.unpriced.contains(&BetType::Wide));
    }

    #[test]
    fn unobserved_bet_type_is_not_reported_as_unpriced() {
        // 取得失敗（＝観測できなかった）は「分からない」であって「売っていない」ではない。
        // マークすると一過性の取得失敗が TTL のあいだ取り直されなくなり、#294 の自己修復が鈍る。
        let race_id = RaceId::try_from("2026-1-hakodate-6-5R").unwrap();
        let mut observed = all_exotic();
        observed.remove(&BetType::Trio); // 三連複だけ取得に失敗した状況
        let exotic = FetchedExoticOdds {
            observed,
            ..FetchedExoticOdds::default()
        };

        let scraped = assemble_netkeiba(&FetchedOdds::default(), &exotic, race_id);
        assert!(!scraped.unpriced.contains(&BetType::Trio));
        // 観測できた他券種は「取得成功して 0 行」なので未発売と確認できている。
        assert!(scraped.unpriced.contains(&BetType::Quinella));
    }

    #[test]
    fn observed_or_skipped_maps_success_to_some_and_failure_to_none() {
        // **極性の回帰ガード**。ここが反転すると「取得に失敗した券種だけが観測済み」になり、
        // (a) 未発売のマークが 1 件も作られず #632 の修正が本番経路で効かない
        // (b) 一過性の取得失敗が「未発売」として TTL のあいだ固定され #294 の自己修復が壊れる
        // という、本 PR の主張を両方向に裏返す状態になる（実際に 1 巡目でこれを作り込んだ）。
        assert_eq!(
            observed_or_skipped(Ok(vec![1, 2, 3]), "202601020605", 7),
            Some(vec![1, 2, 3]),
            "取得成功は観測できた"
        );
        assert_eq!(
            observed_or_skipped(Ok(Vec::<i32>::new()), "202601020605", 7),
            Some(Vec::new()),
            "取得成功で 0 行は「未発売と確認できた」＝観測できた"
        );
        assert_eq!(
            observed_or_skipped(Err(Error::Fetch("boom".into())), "202601020605", 7),
            None::<Vec<i32>>,
            "取得失敗は観測できていない"
        );
    }

    #[test]
    fn record_observed_marks_only_successful_fetches() {
        // `fetch_one_exotic` の結果 → `observed` の配線。1 巡目のバグはここを通るテストが
        // 無かったために素通りした（assemble_netkeiba のテストは observed を手組みしていた）。
        let mut observed = BTreeSet::new();
        let rows = record_observed(Some(vec![10]), BetType::Trio, &mut observed);
        assert_eq!(rows, vec![10]);
        assert_eq!(observed, BTreeSet::from([BetType::Trio]));

        let rows = record_observed(None::<Vec<i32>>, BetType::Wide, &mut observed);
        assert!(
            rows.is_empty(),
            "取得失敗は空 Vec に倒す（他券種を巻き添えにしない）"
        );
        assert_eq!(
            observed,
            BTreeSet::from([BetType::Trio]),
            "取得失敗した券種は観測済みに入れない"
        );
    }

    #[test]
    fn default_exotic_input_observes_no_unpriced_bet_type() {
        // 単複のみの取得（odds-collect）や組合せ取得の丸ごと失敗は組合せ券種を一度も見ていない。
        // `FetchedExoticOdds::default()` は observed が空＝「何も観測していない」なので、
        // 未発売マークを作らない。**既定値が最も安全な解釈になっている**ことの回帰ガード
        // （failed 集合を持つ設計だと既定値が「全券種未発売」という最も危険な解釈になる）。
        let race_id = RaceId::try_from("2026-1-hakodate-6-5R").unwrap();
        let scraped = assemble_netkeiba(
            &FetchedOdds::default(),
            &FetchedExoticOdds::default(),
            race_id,
        );
        assert!(scraped.unpriced.is_empty());
    }

    #[tokio::test]
    async fn scrape_propagates_conversion_error_for_non_jra_race_id() {
        // scrape() の glue 回帰ガード: 馬個別成績由来の合成 race_id `nk-<12桁>` は paddock RaceId の
        // `{year}-{round}-{slug}-{day}-{race_num}R` 構造でなく（末尾 R も無い）、
        // netkeiba_race_id_from_paddock が変換段で Err を返してネットワークへ出る前に伝播する
        // （OddsInteractor 側で skip(None) になる）。ネットワーク非依存で変換分岐のみを検証する。
        // #458 で scrape() は spawn_blocking 経由の async になったため await して検証する。
        let scraper = UreqNetkeibaScraper::new();
        let synthetic = RaceId::try_from("nk-202602010605").unwrap();
        assert!(scraper.scrape(&synthetic).await.is_err());
    }

    // --- transient リトライ（#288） ---------------------------------------
    // jra-fetcher（ADR 0021/0029）の TcpListener ベース局所サーバ方式を踏襲し、netkeiba GET の
    // `call_with_retry` が一時障害を再試行して回復することを実機ソケットで検証する。

    // `Read` はモジュール先頭の `use std::io::Read;` が `super::*` 経由で入るため再 import 不要。
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread::{self, JoinHandle};

    /// `responses` を接続ごとに 1 つずつ順に返す最小 HTTP サーバ。返り値は URL・受理接続数・join handle。
    fn serve(responses: Vec<&'static str>) -> (String, Arc<AtomicUsize>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&count);
        let handle = thread::spawn(move || {
            for resp in responses {
                let (mut stream, _) = listener.accept().unwrap();
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                stream.write_all(resp.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
        });
        (format!("http://{addr}/odds.json"), count, handle)
    }

    const R_503: &str =
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    const R_200_OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
    const R_404: &str = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    fn test_agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .build()
            .into()
    }

    #[test]
    fn call_with_retry_retries_transient_5xx_then_succeeds() {
        // 503, 503, 200 → 2 回再試行して本文を返す。
        let (url, count, handle) = serve(vec![R_503, R_503, R_200_OK]);
        let resp = call_with_retry(&test_agent(), &url).expect("should succeed after retries");
        let mut body = String::new();
        resp.into_body()
            .into_reader()
            .read_to_string(&mut body)
            .unwrap();
        assert_eq!(body, "ok");
        assert_eq!(
            count.load(Ordering::SeqCst),
            3,
            "expected 3 attempts (2 retries) before success"
        );
        handle.join().unwrap();
    }

    #[test]
    fn call_with_retry_gives_up_after_max_attempts_on_persistent_5xx() {
        // 毎回 503 → 共通 MAX_ATTEMPTS(3) 回で打ち切り Err（無限ループしない）。
        // 回数は scraper_util に一元化した共通ポリシー（#460）。
        const MAX_ATTEMPTS: usize = 3;
        let (url, count, handle) = serve(vec![R_503; MAX_ATTEMPTS]);
        assert!(
            call_with_retry(&test_agent(), &url).is_err(),
            "persistent 5xx must surface as an error, not hang or succeed"
        );
        assert_eq!(
            count.load(Ordering::SeqCst),
            MAX_ATTEMPTS,
            "should attempt exactly MAX_ATTEMPTS times then give up"
        );
        handle.join().unwrap();
    }

    #[test]
    fn call_with_retry_does_not_retry_4xx() {
        // 404 は非 transient: 1 回で即 Err（再試行しない）。
        let (url, count, handle) = serve(vec![R_404]);
        assert!(call_with_retry(&test_agent(), &url).is_err());
        assert_eq!(count.load(Ordering::SeqCst), 1, "4xx must not be retried");
        handle.join().unwrap();
    }
}
