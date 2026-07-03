use std::cmp::Ordering;

use chrono::NaiveDate;

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{LiveEvRepository, LiveEvSnapshot};

/// `GET /api/live/{date}` が返すライブ EV ビュー（use-case 側の view 型）。
/// slip 伝票は JSON テキストのまま運び、rest-controller の DTO でデシリアライズする
/// （use-case を serde 非依存に保つ）。
#[derive(Debug, Clone)]
pub struct LiveView {
    pub date: NaiveDate,
    pub summary: LiveSummary,
    pub races: Vec<LiveRaceView>,
}

/// 一望サマリ（張る本数・監視数・最終更新時刻）。
#[derive(Debug, Clone)]
pub struct LiveSummary {
    /// 最新サイクルが `verdict='bet'` の race 数。
    pub bet_race_count: u32,
    /// 監視レース数（= `races.len()`）。
    pub watched_race_count: u32,
    /// 全 race 中の最新 `captured_at` の最大値（辞書順＝時刻順）。無ければ `None`。
    pub last_updated: Option<String>,
}

/// 1 レースの最新サイクル本体＋フリップ。
#[derive(Debug, Clone)]
pub struct LiveRaceView {
    pub race_id: String,
    pub venue: String,
    pub race_no: u32,
    pub post_time: Option<String>,
    pub captured_at: String,
    pub verdict: String,
    pub roi: f64,
    pub konsen: bool,
    pub axis: u32,
    pub axis_prob: f64,
    pub axis_win_odds: Option<f64>,
    pub odds_missing: bool,
    /// 買い目伝票 JSONB（`slip` 列）の JSON テキスト。
    pub slip_json: String,
    pub flip: LiveFlip,
}

/// 直前サイクルとの差分（◎変化・+EV↔−EV 反転）。直前が無ければ全て false / None。
#[derive(Debug, Clone)]
pub struct LiveFlip {
    pub axis_changed: bool,
    pub prev_axis: Option<u32>,
    pub ev_reversed: bool,
    pub prev_verdict: Option<String>,
    pub prev_roi: Option<f64>,
}

impl<R: LiveEvRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 指定開催日の race ごと最新サイクル＋伝票を返す（read-only, #260 / ADR 0064）。
    ///
    /// repository はフラットな `rank<=2` 行（`(race_id, rank)` 昇順）を返すので、ここで
    /// 最新（`rank=1`）/直前（`rank=2`）にグルーピングし、フリップと一望サマリを算出する。
    /// 該当行が無ければ races 空・count 0・`last_updated=None`（200 空・404 にしない）。
    pub async fn find_live_by_date(&self, date: NaiveDate) -> Result<LiveView> {
        let rows = self.repository.find_live_ev_by_date(date).await?;

        let mut races: Vec<LiveRaceView> = Vec::new();
        let mut i = 0;
        while i < rows.len() {
            // 行は (race_id, rank) 昇順のため rows[i] は必ず rank=1（最新）。
            let latest = &rows[i];
            // 直後の行が同一 race_id なら rank=2（直前サイクル）。
            let prev = rows
                .get(i + 1)
                .filter(|next| next.race_id == latest.race_id);

            races.push(build_race_view(latest, prev));
            i += if prev.is_some() { 2 } else { 1 };
        }

        let summary = LiveSummary {
            bet_race_count: races.iter().filter(|r| r.verdict == "bet").count() as u32,
            watched_race_count: races.len() as u32,
            last_updated: races.iter().map(|r| r.captured_at.clone()).max(),
        };

        // post_time 昇順（NULL は後ろ）→ race_no 昇順。post_time は同規約の rfc3339 文字列のため
        // 辞書順比較が時刻順比較になる。
        races.sort_by(|a, b| match (&a.post_time, &b.post_time) {
            (Some(x), Some(y)) => x.cmp(y).then(a.race_no.cmp(&b.race_no)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => a.race_no.cmp(&b.race_no),
        });

        Ok(LiveView {
            date,
            summary,
            races,
        })
    }
}

/// 最新行＋（あれば）直前行から 1 レースのビューを組み立てる。
fn build_race_view(latest: &LiveEvSnapshot, prev: Option<&LiveEvSnapshot>) -> LiveRaceView {
    let flip = match prev {
        Some(p) => LiveFlip {
            axis_changed: p.axis != latest.axis,
            prev_axis: Some(p.axis),
            ev_reversed: p.verdict != latest.verdict,
            prev_verdict: Some(p.verdict.clone()),
            prev_roi: Some(p.roi),
        },
        None => LiveFlip {
            axis_changed: false,
            prev_axis: None,
            ev_reversed: false,
            prev_verdict: None,
            prev_roi: None,
        },
    };

    LiveRaceView {
        race_id: latest.race_id.clone(),
        venue: latest.venue.clone(),
        race_no: latest.race_no,
        post_time: latest.post_time.clone(),
        captured_at: latest.captured_at.clone(),
        verdict: latest.verdict.clone(),
        roi: latest.roi,
        konsen: latest.konsen,
        axis: latest.axis,
        axis_prob: latest.axis_prob,
        axis_win_odds: latest.axis_win_odds,
        odds_missing: latest.odds_missing,
        slip_json: latest.slip_json.clone(),
        flip,
    }
}
