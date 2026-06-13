use chrono::Utc;
use paddock_domain::{HorseEntry, RaceCard, RaceId};

use crate::error::Result;
use crate::interactor::card::CardInteractor;
use crate::netkeiba_scraper::NetkeibaScraper;
use crate::repository::{FetchRecord, OddsRow, RaceOddsRecord, Repository};

/// 出馬表・オッズ取り込みの結果サマリ。
#[derive(Debug, Clone, PartialEq)]
pub struct IngestCardResponse {
    /// 出馬表（カード）を保存したか。重複スキップ時は false。
    pub card_saved: bool,
    /// 保存した出走馬数（カードをスキップした場合は 0）。
    pub entries_saved: usize,
    /// 保存したオッズ行数（単勝・複勝＋馬連・馬単・三連複・三連単。レース前で未確定なら 0）。
    pub odds_saved: usize,
}

impl<R: Repository, S: NetkeibaScraper> CardInteractor<R, S> {
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
        if already && !force {
            tracing::info!(%netkeiba_id, "card already fetched, skipping (use --force to refetch)");
        } else {
            let fetched = self.scraper.fetch_card(netkeiba_id)?;
            let entries: Vec<HorseEntry> = fetched
                .entries
                .into_iter()
                .map(|e| HorseEntry {
                    gate_num: e.gate_num,
                    horse_num: e.horse_num,
                    horse_name: e.horse_name,
                    jockey: e.jockey,
                    trainer: e.trainer,
                })
                .collect();
            entries_saved = entries.len();
            let card = RaceCard {
                race_id: race_id.clone(),
                date: fetched.date,
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
        //    単勝・複勝(type=1) は 1 レスポンスで両方、組合せ券種(type=4/6/7/8) は別 API で取得し、
        //    全券種を 1 レコードにまとめて保存する（#102。キー規約は各ドメイン型の to_key）。
        let odds = self.scraper.fetch_win_place_odds(netkeiba_id)?;
        let exotic = self.scraper.fetch_exotic_odds(netkeiba_id)?;
        let mut rows: Vec<OddsRow> = Vec::new();
        rows.extend(
            odds.win
                .iter()
                .map(|w| OddsRow::win(w.horse_num.value(), w.odds, w.popularity)),
        );
        rows.extend(
            odds.place
                .iter()
                .map(|p| OddsRow::place(p.horse_num.value(), p.odds_low, p.odds_high, p.popularity)),
        );
        rows.extend(
            exotic
                .quinella
                .iter()
                .map(|q| OddsRow::quinella(q.combination, q.odds)),
        );
        rows.extend(
            exotic
                .exacta
                .iter()
                .map(|e| OddsRow::exacta(e.combination, e.odds)),
        );
        rows.extend(exotic.trio.iter().map(|t| OddsRow::trio(t.combination, t.odds)));
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
        })
    }
}
