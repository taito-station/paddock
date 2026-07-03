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
    /// repository はフラットな `rank<=2` 行（`(race_id, rank)` 昇順）を返すので、
    /// [`assemble_live_view`] が最新（`rank=1`）/直前（`rank=2`）へグルーピングし、フリップと
    /// 一望サマリを算出する。該当行が無ければ races 空・count 0・`last_updated=None`（200 空・404 にしない）。
    pub async fn find_live_by_date(&self, date: NaiveDate) -> Result<LiveView> {
        let rows = self.repository.find_live_ev_by_date(date).await?;
        Ok(assemble_live_view(date, rows))
    }
}

/// repository が返す `rank<=2` のフラット行を最新/直前へグルーピングし、フリップ・一望サマリ・
/// 整列まで行う純関数（DB 非依存＝ユニットテスト可能）。行は `(race_id, rank)` 昇順を前提とする。
fn assemble_live_view(date: NaiveDate, rows: Vec<LiveEvSnapshot>) -> LiveView {
    let mut races: Vec<LiveRaceView> = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        // 行は (race_id, rank) 昇順のため rows[i] は必ず rank=1（最新）。
        let latest = &rows[i];
        // 直後の行が同一 race_id かつ rank=2 なら直前サイクル（フリップ算出に使う）。
        let prev = rows
            .get(i + 1)
            .filter(|next| next.race_id == latest.race_id && next.rank == 2);

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

    LiveView {
        date,
        summary,
        races,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用 snapshot ビルダ。rank・race_id・verdict・roi・axis・captured_at・post_time を指定。
    #[allow(clippy::too_many_arguments)]
    fn snap(
        rank: u32,
        race_id: &str,
        race_no: u32,
        verdict: &str,
        roi: f64,
        axis: u32,
        captured_at: &str,
        post_time: Option<&str>,
    ) -> LiveEvSnapshot {
        LiveEvSnapshot {
            rank,
            race_id: race_id.into(),
            venue: "tokyo".into(),
            race_no,
            post_time: post_time.map(Into::into),
            captured_at: captured_at.into(),
            verdict: verdict.into(),
            roi,
            konsen: false,
            axis,
            axis_prob: 30.0,
            axis_win_odds: Some(2.0),
            odds_missing: false,
            slip_json: r#"{"race_budget":5000,"legs":[]}"#.into(),
        }
    }

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, 20).unwrap()
    }

    #[test]
    fn empty_rows_yield_empty_view() {
        let v = assemble_live_view(date(), vec![]);
        assert!(v.races.is_empty());
        assert_eq!(v.summary.bet_race_count, 0);
        assert_eq!(v.summary.watched_race_count, 0);
        assert_eq!(v.summary.last_updated, None);
    }

    #[test]
    fn single_cycle_has_no_flip() {
        let rows = vec![snap(
            1,
            "r1",
            11,
            "bet",
            120.0,
            5,
            "2026-06-20T10:00:00Z",
            None,
        )];
        let v = assemble_live_view(date(), rows);
        assert_eq!(v.races.len(), 1);
        let r = &v.races[0];
        assert_eq!(r.verdict, "bet");
        assert!(!r.flip.axis_changed && !r.flip.ev_reversed);
        assert_eq!(r.flip.prev_axis, None);
        assert_eq!(r.flip.prev_verdict, None);
        assert_eq!(r.flip.prev_roi, None);
        assert_eq!(v.summary.bet_race_count, 1);
    }

    #[test]
    fn two_cycles_compute_flip_from_previous() {
        // 最新(rank1)=bet/axis5、直前(rank2)=skip/axis3 → 軸変化＋EV反転。
        let rows = vec![
            snap(1, "r1", 11, "bet", 120.0, 5, "2026-06-20T10:20:00Z", None),
            snap(2, "r1", 11, "skip", 90.0, 3, "2026-06-20T10:00:00Z", None),
        ];
        let v = assemble_live_view(date(), rows);
        assert_eq!(v.races.len(), 1);
        let r = &v.races[0];
        // 本体は最新サイクル。
        assert_eq!(r.captured_at, "2026-06-20T10:20:00Z");
        assert_eq!(r.verdict, "bet");
        assert_eq!(r.axis, 5);
        // フリップは直前との差分。
        assert!(r.flip.axis_changed);
        assert_eq!(r.flip.prev_axis, Some(3));
        assert!(r.flip.ev_reversed);
        assert_eq!(r.flip.prev_verdict, Some("skip".into()));
        assert_eq!(r.flip.prev_roi, Some(90.0));
    }

    #[test]
    fn multiple_races_group_and_summarize() {
        // r1 は 2 サイクル、r2 は 1 サイクルのみ。summary と grouping を検証。
        let rows = vec![
            snap(
                1,
                "r1",
                11,
                "bet",
                120.0,
                5,
                "2026-06-20T10:20:00Z",
                Some("2026-06-20T15:40:00Z"),
            ),
            snap(
                2,
                "r1",
                11,
                "skip",
                90.0,
                5,
                "2026-06-20T10:00:00Z",
                Some("2026-06-20T15:40:00Z"),
            ),
            snap(
                1,
                "r2",
                9,
                "skip",
                80.0,
                2,
                "2026-06-20T10:10:00Z",
                Some("2026-06-20T15:10:00Z"),
            ),
        ];
        let v = assemble_live_view(date(), rows);
        assert_eq!(v.races.len(), 2);
        assert_eq!(v.summary.bet_race_count, 1); // r1 のみ bet
        assert_eq!(v.summary.watched_race_count, 2);
        assert_eq!(v.summary.last_updated, Some("2026-06-20T10:20:00Z".into())); // 全 race 中の最新
        // post_time 昇順（r2 15:10 が先、r1 15:40 が後）。
        assert_eq!(v.races[0].race_id, "r2");
        assert_eq!(v.races[1].race_id, "r1");
        // r1 は axis 不変（5→5）で ev 反転のみ。
        assert!(!v.races[1].flip.axis_changed && v.races[1].flip.ev_reversed);
        // r2 は直前なし。
        assert_eq!(v.races[0].flip.prev_axis, None);
    }

    #[test]
    fn post_time_null_sorts_after_present() {
        let rows = vec![
            snap(
                1,
                "r_null",
                12,
                "skip",
                80.0,
                1,
                "2026-06-20T10:00:00Z",
                None,
            ),
            snap(
                1,
                "r_have",
                1,
                "skip",
                80.0,
                1,
                "2026-06-20T10:00:00Z",
                Some("2026-06-20T15:00:00Z"),
            ),
        ];
        let v = assemble_live_view(date(), rows);
        // post_time あり → null の順。
        assert_eq!(v.races[0].race_id, "r_have");
        assert_eq!(v.races[1].race_id, "r_null");
    }
}
