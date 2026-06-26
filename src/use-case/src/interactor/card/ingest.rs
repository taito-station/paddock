use chrono::Utc;
use paddock_domain::{HorseEntry, RaceCard, RaceId};

use crate::error::Result;
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

        // 2. オッズ: 常に取得。確定前で空なら保存をスキップ（後で再実行して取り直す想定）。
        //    単勝・複勝(type=1) は 1 レスポンスで両方、組合せ券種(type=4/5/6/7/8) は別 API で取得し、
        //    全券種を 1 レコードにまとめて保存する（#102。キー規約は各ドメイン型の to_key）。
        //    単複もベストエフォート: 前日(status=yoso)など未発売・想定外 status で取得失敗しても、
        //    本コマンドの主目的（card 保存と後続の近走取り込み #103）を巻き添えにしない。
        //    オッズ無し＝EV/Kelly が出ないだけで、当日朝に再取得すればよい（fail-closed な
        //    #100 status ゲート自体は据え置き＝yoso オッズは保存しない）。
        //    yoso だけでなくネットワーク断・5xx 等の想定外失敗も同様に握り潰すのは意図的:
        //    odds は実行ごとに取り直す揮発データで永続欠落にはならず、card+近走（再取得が
        //    高コストで主目的）を odds の一時障害で止めない方が運用上正しいため。warn は残す。
        let odds = self.scraper.fetch_win_place_odds(netkeiba_id).unwrap_or_else(|e| {
            tracing::warn!(%netkeiba_id, error = %e, "単複オッズの取得に失敗（未発売/想定外status等）、オッズ無しで card+近走取り込みを継続");
            Default::default()
        });
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
        rows.extend(
            odds.place.iter().map(|p| {
                OddsRow::place(p.horse_num.value(), p.odds_low, p.odds_high, p.popularity)
            }),
        );
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

        Ok(IngestCardResponse {
            card_saved,
            entries_saved,
            odds_saved,
            horse_ids,
        })
    }
}
