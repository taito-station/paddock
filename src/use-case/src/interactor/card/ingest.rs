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
    /// 保存したオッズ行数（単勝+複勝。レース前で未確定なら 0）。
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
        //    netkeiba は単勝と複勝を 1 レスポンスで返すため一括取得し、両方を 1 レコードに詰める。
        let odds = self.scraper.fetch_win_place_odds(netkeiba_id)?;
        let odds_saved = odds.win.len() + odds.place.len();
        if odds.win.is_empty() && odds.place.is_empty() {
            tracing::info!(%netkeiba_id, "win/place odds not available yet, skipping odds save");
        } else {
            // 単勝・複勝とも combination_key は素の馬番文字列("1".."18")。組合せ券種(#38)は
            // 別途キー規約(昇順連結等)を定義する。ゼロ詰めはしない。
            let mut rows: Vec<OddsRow> = Vec::with_capacity(odds_saved);
            rows.extend(odds.win.iter().map(|w| OddsRow {
                bet_type: "win".to_string(),
                combination_key: w.horse_num.value().to_string(),
                odds: w.odds,
                odds_high: None,
                popularity: w.popularity,
            }));
            // 複勝は幅 odds。下限を `odds`、上限を `odds_high` に入れる。
            rows.extend(odds.place.iter().map(|p| OddsRow {
                bet_type: "place".to_string(),
                combination_key: p.horse_num.value().to_string(),
                odds: p.odds_low,
                odds_high: Some(p.odds_high),
                popularity: p.popularity,
            }));
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
