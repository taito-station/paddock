use chrono::Utc;
use paddock_domain::{HorseEntry, RaceCard, RaceId};

use crate::error::{Error, Result};
use crate::interactor::card::CardInteractor;
use crate::netkeiba_scraper::NetkeibaScraper;
use crate::repository::{
    FetchRecord, FetchRepository, OddsRepository, OddsRow, RaceCardRepository, RaceOddsRecord,
};

/// 出馬表・オッズ取り込みの結果サマリ。
#[derive(Debug, Clone, PartialEq)]
pub struct IngestCardResponse {
    /// 出馬表（カード）を保存したか。重複スキップ時は false。
    pub card_saved: bool,
    /// 保存した出走馬数（カードをスキップした場合は 0）。
    pub entries_saved: usize,
    /// 保存したオッズ行数（単勝・複勝＋馬連・ワイド・馬単・三連複・三連単。レース前で未確定なら 0）。
    pub odds_saved: usize,
    /// 取得した出走各馬の netkeiba horse_id（近走取り込み #103 の再利用キー）。
    /// カードをスキップ（取得済み）した場合は空。呼び出し側はこれで出馬表の再取得を避ける。
    pub horse_ids: Vec<String>,
    /// 単複(type=1)が transient なネットワーク障害（接続リセット等）でリトライ後も取得できず、
    /// オッズを保存しなかったか（#288）。未発売(status=yoso)による空とは区別する。true のとき
    /// 呼び出し側（bin）は専用 exit code で「単複だけ未取得・要再取得」を surface する。
    pub win_odds_degraded: bool,
}

impl<R: RaceCardRepository + OddsRepository + FetchRepository, S: NetkeibaScraper>
    CardInteractor<R, S>
{
    /// netkeiba の出馬表と単勝オッズを取得し、`race_cards`/`horse_entries`/`race_odds` に保存する。
    ///
    /// カードは `fetch_history`（source_key `netkeiba-card:<id>`）で重複取得を抑止する
    /// （`force=true` で再取得）。オッズは確定値が時間で変わるため毎回取得・上書きする。
    pub async fn ingest(
        &self,
        netkeiba_id: &str,
        race_id: RaceId,
        force: bool,
    ) -> Result<IngestCardResponse> {
        let source_key = format!("netkeiba-card:{netkeiba_id}");
        // 1 回の取り込みの時刻。カード履歴とオッズで同じ値を使い、両者の fetched_at を揃える。
        let now = Utc::now();

        // 1. カード: 取得済みかつ !force ならスキップ。そうでなければ取得・保存して履歴を記録。
        let already = self.repo.fetch_history_contains(&source_key).await?;
        let (mut card_saved, mut entries_saved) = (false, 0);
        // カードを取得したときのみ horse_id を採れる。再利用キーとして呼び出し側へ返す
        // （取得済みスキップ時は空のまま → 呼び出し側は出馬表から引き直す）。
        let mut horse_ids: Vec<String> = Vec::new();
        if already && !force {
            tracing::info!(%netkeiba_id, "card already fetched, skipping (use --force to refetch)");
        } else {
            let fetched = self.scraper.fetch_card(netkeiba_id)?;
            horse_ids = fetched
                .entries
                .iter()
                .filter_map(|e| e.horse_id.as_ref().map(|id| id.value().to_string()))
                .collect();
            let entries: Vec<HorseEntry> = fetched
                .entries
                .into_iter()
                .map(|e| HorseEntry {
                    gate_num: e.gate_num,
                    horse_num: e.horse_num,
                    horse_name: e.horse_name,
                    jockey: e.jockey,
                    trainer: e.trainer,
                    weight_carried: e.weight_carried,
                })
                .collect();
            entries_saved = entries.len();
            let card = RaceCard {
                race_id: race_id.clone(),
                date: fetched.date,
                post_time: fetched.post_time,
                venue: fetched.venue,
                round: fetched.round,
                day: fetched.day,
                race_num: fetched.race_num,
                surface: fetched.surface,
                distance: fetched.distance,
                race_class: fetched.race_class,
                race_name: fetched.race_name,
                entries,
            };
            self.repo.save_race_card(&card).await?;
            self.repo
                .record_fetch(&FetchRecord {
                    source_key,
                    // 取得元の論理識別子。具体的な HTTP URL の組み立ては interface 層
                    // (scraper) の責務なので、use-case はここで netkeiba の URL 形式を持たない。
                    url: format!("netkeiba:shutuba:{netkeiba_id}"),
                    races_saved: 1,
                    horses_saved: entries_saved as u32,
                    fetched_at: now,
                })
                .await?;
            card_saved = true;
        }

        // 2. オッズ: 単勝・複勝(type=1) は 1 レスポンスで両方、組合せ券種(type=4/5/6/7/8) は別 API。
        //    全券種を 1 レコードにまとめて保存する（#102。キー規約は各ドメイン型の to_key）。
        //    単複の取得失敗は 2 種類を区別する（#288）:
        //    - 未発売(status=yoso 等の想定外 status = `Internal`): 正規の「まだオッズが無い」状態。
        //      従来どおり best-effort で握り潰す。card+近走（再取得が高コストで主目的）を巻き添えに
        //      しない。当日朝に再取得すればよい（fail-closed な #100 status ゲートは据え置き）。
        //    - ネットワーク/HTTP 失敗(`Fetch`/`Timeout` = 接続リセット os error 54・タイムアウト・5xx、
        //      および稀な 4xx。scraper 内リトライ後も残る取得失敗): 「本来取れるはずが取れていない」ので
        //      握り潰さず degraded として surface する。win 欠落のまま exotic だけ保存すると predict が
        //      「オッズ有り・win 無し」で誤判定し対象レースが脱落するため、odds 保存自体をスキップする
        //      （部分スナップショットを永続化しない。cf. #287/commit a54e56b）。4xx は再試行しない
        //      （`is_transient` 参照）が、部分永続化を避け非0 exit で surface する点は同じで安全
        //      （消費側の再取得は run/sweep 単位で有界）。
        let mut win_odds_degraded = false;
        let odds = match self.scraper.fetch_win_place_odds(netkeiba_id) {
            Ok(odds) => odds,
            // `Timeout` は防御的: 現状 netkeiba 経路はタイムアウトも `call_with_retry` で
            // `Error::Fetch` に集約するため実際には `Fetch` のみ届くが、将来 Timeout を分けて
            // 伝播しても transient として同じ degraded 分岐に乗るようまとめて捕捉する。
            Err(Error::Fetch(_) | Error::Timeout(_)) => {
                tracing::warn!(%netkeiba_id, "単複オッズの取得に失敗し、オッズ未保存で degraded 継続（card+近走は保存、要再取得）");
                win_odds_degraded = true;
                Default::default()
            }
            // 未発売(status=yoso 等)/想定外 status は best-effort。下の else 分岐で exotic を取りに行き、
            // 取れた分だけ保存しうる（exotic-only の部分スナップショットになりうる）。これは degraded 側で
            // 排除した「win 欠落の部分永続化」と同型に見えるが、netkeiba は単複と組合せをほぼ同時発売する
            // ため win=未発売のときは exotic も未発売（空）→ rows 空 → 保存スキップとなり実質到達しない。
            // よって Parse 経路に degraded 同等の保存スキップ強制は課さない（既存挙動を温存）。
            Err(e) => {
                tracing::warn!(%netkeiba_id, error = %e, "単複オッズの取得に失敗（未発売/想定外status）、オッズ無しで card+近走取り込みを継続");
                Default::default()
            }
        };

        // degraded（単複 transient 失敗）時は win 欠落の部分スナップショットを残さない。
        // exotic も取りに行かず（無駄な 5 リクエストを避ける）、オッズ保存をまるごとスキップする。
        let odds_saved = if win_odds_degraded {
            0
        } else {
            // 組合せ券種はベストエフォート。別 API を 4 本叩くため、その一部が失敗しても
            // 確定済みの単複保存まで巻き添えにしない（取りこぼし耐性、#102 レビュー反映）。
            let exotic = self
                .scraper
                .fetch_exotic_odds(netkeiba_id)
                .unwrap_or_else(|e| {
                    tracing::warn!(%netkeiba_id, error = %e, "組合せ券種オッズの取得に失敗、単複のみ保存して継続");
                    Default::default()
                });
            let mut rows: Vec<OddsRow> = Vec::with_capacity(
                odds.win.len()
                    + odds.place.len()
                    + exotic.quinella.len()
                    + exotic.wide.len()
                    + exotic.exacta.len()
                    + exotic.trio.len()
                    + exotic.trifecta.len(),
            );
            rows.extend(
                odds.win
                    .iter()
                    .map(|w| OddsRow::win(w.horse_num.value(), w.odds, w.popularity)),
            );
            rows.extend(odds.place.iter().map(|p| {
                OddsRow::place(p.horse_num.value(), p.odds_low, p.odds_high, p.popularity)
            }));
            rows.extend(
                exotic
                    .quinella
                    .iter()
                    .map(|q| OddsRow::quinella(q.combination, q.odds)),
            );
            rows.extend(
                exotic
                    .wide
                    .iter()
                    .map(|w| OddsRow::wide(w.combination, w.odds_low, w.odds_high)),
            );
            rows.extend(
                exotic
                    .exacta
                    .iter()
                    .map(|e| OddsRow::exacta(e.combination, e.odds)),
            );
            rows.extend(
                exotic
                    .trio
                    .iter()
                    .map(|t| OddsRow::trio(t.combination, t.odds)),
            );
            rows.extend(
                exotic
                    .trifecta
                    .iter()
                    .map(|t| OddsRow::trifecta(t.combination, t.odds)),
            );
            let odds_saved = rows.len();
            if rows.is_empty() {
                tracing::info!(%netkeiba_id, "odds not available yet, skipping odds save");
            } else {
                self.repo
                    .save_race_odds(&RaceOddsRecord {
                        race_id,
                        fetched_at: now,
                        rows,
                    })
                    .await?;
            }
            odds_saved
        };

        Ok(IngestCardResponse {
            card_saved,
            entries_saved,
            odds_saved,
            horse_ids,
            win_odds_degraded,
        })
    }
}
