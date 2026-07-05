//! 1レース全頭「盤」ビュー（予想ビューア RaceBoard の read ユースケース）。
//!
//! 予想ビューアの1レース盤に必要な全頭データを 1 レスポンスに束ねる read ユースケース。
//! 確率・買い目は既存経路（`predict_race_views` ＋ `build_portfolio`）を**そのまま再利用**し、
//! ここでは表示用の派生値（市場implied・人気順・印・乖離/重なり・混戦）を組むだけ。
//! 買い目の相手選定（top5・`PortfolioConfig::default`）は変えない（CLAUDE.md「相手を top5 より
//! 広げない」・105R backtest 知見を尊重）。全頭は truncate せず返し、top5 から漏れる市場人気馬も
//! 盤で見えるようにする（人間が手動で拾う運用を支える）。

use chrono::NaiveDate;

use paddock_domain::{
    HorseProbability, Mark, Portfolio, PortfolioConfig, RaceId, RaceOdds, TrackCondition,
    build_portfolio,
};

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{OddsRepository, RaceCardRepository, StatsRepository};

/// 混戦しきい値（CLAUDE.md 買い方ルール）。◎の勝率 × この係数以上の馬が ◎含めて
/// [`CONFUSION_MIN_HORSES`] 頭以上いれば混戦。
const CONFUSION_RATIO: f64 = 0.70;
const CONFUSION_MIN_HORSES: u32 = 4;

/// 乖離馬（妙味・ワイドボックス候補）の判定に使う「人気順 − モデル順」の下限。
/// モデル上位圏（[`VALUE_MODEL_RANK_MAX`] 位以内）かつ市場人気がこれ以上下なら乖離馬。
const VALUE_RANK_GAP: i32 = 3;
/// 妙味候補として扱うモデル順位の上限（＝contender 圏）。top3 に限らず 4〜6 位の過小人気も
/// 拾えるようにする（top3 のみだと ◎○▲ と重なり ☆ が機械印として出番を失うため）。
const VALUE_MODEL_RANK_MAX: u32 = 6;

/// 盤全体（1レース）。
#[derive(Debug, Clone)]
pub struct RaceBoard {
    pub race_id: RaceId,
    pub date: NaiveDate,
    /// 開催場スラッグ（例 `hakodate`）。
    pub venue: String,
    pub race_num: u32,
    /// 芝/ダート（`turf` / `dirt`）。
    pub surface: String,
    pub distance: u32,
    pub field_size: u32,
    /// 発走時刻 `HH:MM`（出馬表 PDF 経路など未取得は `None`）。
    pub post_time: Option<String>,
    /// 買い目ポートフォリオ（保存オッズが無ければ `None`＝買い目は組めない。盤は確率だけで描ける）。
    pub portfolio: Option<Portfolio>,
    pub confusion: Confusion,
    /// 全出走馬（truncate しない）。盤面順（`blended` と同順）。
    pub horses: Vec<BoardHorse>,
}

/// 混戦サマリ（CLAUDE.md の混戦判定を機械化）。
#[derive(Debug, Clone)]
pub struct Confusion {
    pub is_confused: bool,
    /// ◎（モデル勝率1位）の勝率 [0,1]。全頭が空なら 0.0。
    pub axis_win_prob: f64,
    /// 判定しきい値 = `axis_win_prob * CONFUSION_RATIO`。
    pub threshold: f64,
    /// しきい値以上の頭数（◎含む）。
    pub qualifying_count: u32,
}

/// 盤の1頭分。
#[derive(Debug, Clone)]
pub struct BoardHorse {
    /// 枠番（出馬表に無ければ `None`）。
    pub gate_num: Option<u32>,
    pub horse_num: u32,
    pub horse_name: String,
    pub jockey: Option<String>,
    /// 表示用（市場ブレンド α）勝率/連対率/複勝率 [0,1]。
    pub win_prob: f64,
    pub place_prob: f64,
    pub show_prob: f64,
    /// EV 視点（純モデル α=1.0）の勝率 [0,1]。
    pub pure_win_prob: f64,
    /// 市場implied 勝率（フィールド内 `1/単勝` 正規化。単勝未取得なら `None`）。
    pub market_implied: Option<f64>,
    pub win_odds: Option<f64>,
    pub place_odds_low: Option<f64>,
    pub place_odds_high: Option<f64>,
    /// 単勝人気（オッズ昇順ランク・1=1番人気。単勝未取得なら `None`）。乖離判定の市場順位も兼ねる。
    pub popularity: Option<u32>,
    /// モデル勝率順位（1=最上位）。
    pub model_rank: u32,
    /// 機械導出の印スラッグ（honmei/taikou/tanana/hoshi）。無印は `None`。
    pub mark: Option<String>,
    /// 重なり馬（モデル勝率1位 かつ 単勝人気1位＝ほぼ複勝圏サイン）。
    pub is_overlay: bool,
    /// 乖離馬（モデル上位×市場人気低＝妙味・ワイドボックス候補）。
    pub is_value: bool,
}

impl<R: StatsRepository + RaceCardRepository + OddsRepository, P: PdfParser, F: PdfFetcher>
    Interactor<R, P, F>
{
    /// 1レース盤（全頭 ＋ 買い目 ＋ 混戦/乖離/重なり）を組んで返す。
    ///
    /// 確率は [`Self::recommend_bets`] と同経路（軸/相手=blended・EV=pure の #272 循環断ち）。
    /// オッズは保存スナップショット（`find_race_odds(.., None)`）を参照しライブ取得しない。
    /// 出馬表が無ければ `predict_race_views` が `Error::NotFound` を返す。
    pub async fn race_board(
        &self,
        race_id: &RaceId,
        budget: u64,
        blend_alpha: Option<f64>,
        track_condition: Option<TrackCondition>,
    ) -> Result<RaceBoard> {
        let views = self
            .predict_race_views(race_id, blend_alpha, track_condition, false)
            .await?;
        let card = self.race_card(race_id).await?;
        let odds = self.repository.find_race_odds(race_id, None).await?;

        // 買い目は既存経路そのまま（相手 top5 不変）。オッズ無しなら組まない。
        let portfolio = odds.as_ref().map(|o| {
            build_portfolio(
                &views.blended,
                &views.pure,
                o,
                budget,
                &PortfolioConfig::default(),
            )
        });

        let horses = build_board_horses(&views.blended, &views.pure, odds.as_ref(), card.as_ref());
        let confusion = compute_confusion(&views.blended);

        let (venue, surface, distance, race_num, date, post_time) = match card.as_ref() {
            Some(c) => (
                c.venue.as_slug().to_string(),
                c.surface.as_str().to_string(),
                c.distance,
                c.race_num,
                c.date,
                c.post_time.map(|t| t.format("%H:%M").to_string()),
            ),
            // 出馬表が無ければ predict_race_views で NotFound になるのでここには基本到達しない。
            None => (
                String::new(),
                String::new(),
                0,
                0,
                NaiveDate::default(),
                None,
            ),
        };

        Ok(RaceBoard {
            race_id: race_id.clone(),
            date,
            venue,
            race_num,
            surface,
            distance,
            field_size: views.blended.len() as u32,
            post_time,
            portfolio,
            confusion,
            horses,
        })
    }
}

/// 全頭の盤行を組む純関数（IO 非依存・単体テスト可能）。
fn build_board_horses(
    blended: &[HorseProbability],
    pure: &[HorseProbability],
    odds: Option<&RaceOdds>,
    card: Option<&paddock_domain::RaceCard>,
) -> Vec<BoardHorse> {
    // 市場implied・人気（単勝オッズ由来）。
    let (implied, popularity) = compute_market(blended, odds);
    // モデル勝率順位（降順・1始まり）。
    let model_rank = model_ranks(blended);

    blended
        .iter()
        .map(|hp| {
            let num = hp.horse_num.value();
            let pure_win_prob = pure
                .iter()
                .find(|p| p.horse_num == hp.horse_num)
                .map(|p| p.win_prob)
                .unwrap_or(hp.win_prob);
            let entry = card.and_then(|c| c.entries.iter().find(|e| e.horse_num == hp.horse_num));
            let win_odds = odds
                .and_then(|o| o.win.get(&hp.horse_num))
                .map(|v| v.value());
            let place = odds.and_then(|o| o.place.get(&hp.horse_num));
            // model_rank は blended（表示用の勝率と同系統）で算出する。乖離判定を pure で行うと
            // 盤に見える「勝率(blended)」と「妙味フラグ」の基準がズレて読み手が混乱するため、
            // 表示と同じ blended 順位で市場人気(pop)との差を測る（純モデル差の増幅より整合性を優先）。
            let mr = *model_rank.get(&num).unwrap_or(&0);
            let pop = popularity.get(&num).copied();
            // 重なり馬: blended 勝率1位 かつ 単勝人気1位。◎(model_rank==1)＝買い目軸(build_portfolio の
            // rank_axis_partners も blended 首位・同一 tie-break)なので、盤の ◎ と「軸 N」は構造上一致する。
            let is_overlay = mr == 1 && pop == Some(1);
            let is_value = mr <= VALUE_MODEL_RANK_MAX
                && pop.is_some_and(|p| p as i32 - mr as i32 >= VALUE_RANK_GAP);
            let mark = derive_mark(mr, is_value).map(|m| m.as_slug().to_string());

            BoardHorse {
                gate_num: entry.map(|e| e.gate_num.value()),
                horse_num: num,
                horse_name: hp.horse_name.value().to_string(),
                jockey: entry.and_then(|e| e.jockey.as_ref().map(|j| j.value().to_string())),
                win_prob: hp.win_prob,
                place_prob: hp.place_prob,
                show_prob: hp.show_prob,
                pure_win_prob,
                market_implied: implied.get(&num).copied(),
                win_odds,
                place_odds_low: place.map(|p| p.low.value()),
                place_odds_high: place.map(|p| p.high.value()),
                popularity: pop,
                model_rank: mr,
                mark,
                is_overlay,
                is_value,
            }
        })
        .collect()
}

/// 単勝オッズから市場implied（フィールド内 `1/odds` 正規化＝控除抜き）と人気順（昇順・1始まり）を導く。
/// 単勝が 1 頭も無ければ両方空。戻り値は馬番 → 値のマップ。
fn compute_market(
    blended: &[HorseProbability],
    odds: Option<&RaceOdds>,
) -> (
    std::collections::HashMap<u32, f64>,
    std::collections::HashMap<u32, u32>,
) {
    use std::collections::HashMap;
    let mut implied = HashMap::new();
    let mut popularity = HashMap::new();
    let Some(o) = odds else {
        return (implied, popularity);
    };
    // フィールド（出走馬）に限定した単勝オッズを集める。
    let mut priced: Vec<(u32, f64)> = blended
        .iter()
        .filter_map(|hp| {
            o.win
                .get(&hp.horse_num)
                .map(|v| (hp.horse_num.value(), v.value()))
        })
        .collect();
    if priced.is_empty() {
        return (implied, popularity);
    }
    let inv_sum: f64 = priced.iter().map(|(_, win_odds)| 1.0 / win_odds).sum();
    for (num, win_odds) in &priced {
        implied.insert(*num, (1.0 / win_odds) / inv_sum);
    }
    // 人気: オッズ昇順（同値は馬番昇順で安定）。
    priced.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    for (rank, (num, _)) in priced.iter().enumerate() {
        popularity.insert(*num, rank as u32 + 1);
    }
    (implied, popularity)
}

/// モデル勝率の降順順位（1始まり・同率は馬番昇順で安定）。馬番 → 順位。
fn model_ranks(blended: &[HorseProbability]) -> std::collections::HashMap<u32, u32> {
    let mut idx: Vec<(u32, f64)> = blended
        .iter()
        .map(|hp| (hp.horse_num.value(), hp.win_prob))
        .collect();
    idx.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    idx.iter()
        .enumerate()
        .map(|(rank, (num, _))| (*num, rank as u32 + 1))
        .collect()
}

/// 機械印: モデル勝率 1→◎ 2→○ 3→▲、それ以外で乖離馬なら ☆。無印は `None`。
fn derive_mark(model_rank: u32, is_value: bool) -> Option<Mark> {
    match model_rank {
        1 => Some(Mark::Honmei),
        2 => Some(Mark::Taikou),
        3 => Some(Mark::Tanana),
        _ if is_value => Some(Mark::Hoshi),
        _ => None,
    }
}

/// 混戦判定（CLAUDE.md）: ◎（勝率1位）の勝率 × 0.70 以上の馬が ◎含め 4 頭以上。
fn compute_confusion(blended: &[HorseProbability]) -> Confusion {
    let axis_win_prob = blended.iter().map(|hp| hp.win_prob).fold(0.0_f64, f64::max);
    let threshold = axis_win_prob * CONFUSION_RATIO;
    let qualifying_count = blended.iter().filter(|hp| hp.win_prob >= threshold).count() as u32;
    Confusion {
        is_confused: axis_win_prob > 0.0 && qualifying_count >= CONFUSION_MIN_HORSES,
        axis_win_prob,
        threshold,
        qualifying_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paddock_domain::{HorseName, HorseNum};

    fn hp(num: u32, win: f64) -> HorseProbability {
        HorseProbability {
            horse_num: HorseNum::try_from(num).unwrap(),
            horse_name: HorseName::try_from(format!("馬{num}")).unwrap(),
            win_prob: win,
            place_prob: (win * 2.0).min(1.0),
            show_prob: (win * 3.0).min(1.0),
        }
    }

    fn odds_with_win(pairs: &[(u32, f64)]) -> RaceOdds {
        use paddock_domain::OddsValue;
        let mut o = RaceOdds::empty(RaceId::try_from("2026-1-hakodate-8-11R").unwrap());
        for (num, v) in pairs {
            o.win.insert(
                HorseNum::try_from(*num).unwrap(),
                OddsValue::try_from(*v).unwrap(),
            );
        }
        o
    }

    #[test]
    fn model_ranks_desc_stable_by_horse_num() {
        let b = vec![hp(1, 0.10), hp(2, 0.30), hp(3, 0.30)];
        let r = model_ranks(&b);
        assert_eq!(r[&2], 1); // 同率 0.30 は馬番昇順 → 2 が上位
        assert_eq!(r[&3], 2);
        assert_eq!(r[&1], 3);
    }

    #[test]
    fn compute_market_implied_normalizes_and_ranks() {
        let b = vec![hp(1, 0.2), hp(2, 0.2), hp(3, 0.2)];
        let o = odds_with_win(&[(1, 2.0), (2, 4.0), (3, 4.0)]);
        let (implied, pop) = compute_market(&b, Some(&o));
        // 1/2 : 1/4 : 1/4 = 0.5:0.25:0.25 を正規化。
        assert!((implied[&1] - 0.5).abs() < 1e-9);
        assert!((implied[&2] - 0.25).abs() < 1e-9);
        assert_eq!(pop[&1], 1); // 最低オッズ=1番人気
        assert_eq!(pop[&2], 2); // 同値は馬番昇順
        assert_eq!(pop[&3], 3);
    }

    #[test]
    fn overlay_when_model_top_and_pop_top() {
        // 1番: モデル1位 かつ 単勝最低（人気1位）→ overlay。
        let b = vec![hp(1, 0.40), hp(2, 0.30), hp(3, 0.20)];
        let o = odds_with_win(&[(1, 2.0), (2, 3.0), (3, 5.0)]);
        let horses = build_board_horses(&b, &b, Some(&o), None);
        let h1 = horses.iter().find(|h| h.horse_num == 1).unwrap();
        assert!(h1.is_overlay);
        assert_eq!(h1.mark.as_deref(), Some("honmei"));
    }

    #[test]
    fn value_when_model_high_but_market_cheap() {
        // 1番: モデル1位だが単勝が一番高い（人気最下位）→ 乖離馬（is_value）。
        let b = vec![hp(1, 0.40), hp(2, 0.30), hp(3, 0.20), hp(4, 0.10)];
        let o = odds_with_win(&[(1, 20.0), (2, 2.0), (3, 3.0), (4, 4.0)]);
        let horses = build_board_horses(&b, &b, Some(&o), None);
        let h1 = horses.iter().find(|h| h.horse_num == 1).unwrap();
        assert_eq!(h1.model_rank, 1);
        assert_eq!(h1.popularity, Some(4));
        assert!(h1.is_value);
        assert!(!h1.is_overlay);
    }

    #[test]
    fn hoshi_mark_for_value_horse_outside_top3() {
        // ④: モデル4位（top3外）だが単勝最下位（人気8）＝ gap 4 の過小人気 → ☆(hoshi)。
        // top3 は ◎○▲、top3外の乖離馬に ☆ が付くことを担保（旧 top3 上限だと ☆ は到達不能だった）。
        let b = vec![
            hp(1, 0.20),
            hp(2, 0.18),
            hp(3, 0.16),
            hp(4, 0.14),
            hp(5, 0.10),
            hp(6, 0.08),
            hp(7, 0.07),
            hp(8, 0.05),
        ];
        let o = odds_with_win(&[
            (1, 2.0),
            (2, 3.0),
            (3, 4.0),
            (5, 5.0),
            (6, 6.0),
            (7, 7.0),
            (8, 8.0),
            (4, 50.0), // ④ を最も高オッズ＝人気8 に
        ]);
        let horses = build_board_horses(&b, &b, Some(&o), None);
        let h4 = horses.iter().find(|h| h.horse_num == 4).unwrap();
        assert_eq!(h4.model_rank, 4);
        assert_eq!(h4.popularity, Some(8));
        assert!(h4.is_value);
        assert!(!h4.is_overlay);
        assert_eq!(h4.mark.as_deref(), Some("hoshi"));
    }

    #[test]
    fn confusion_true_when_four_within_ratio() {
        // ◎=0.20、しきい値=0.14。0.20/0.16/0.15/0.14 の4頭が該当 → 混戦。
        let b = vec![
            hp(1, 0.20),
            hp(2, 0.16),
            hp(3, 0.15),
            hp(4, 0.14),
            hp(5, 0.05),
        ];
        let c = compute_confusion(&b);
        assert_eq!(c.qualifying_count, 4);
        assert!(c.is_confused);
    }

    #[test]
    fn confusion_false_when_axis_dominant() {
        let b = vec![hp(1, 0.50), hp(2, 0.10), hp(3, 0.08), hp(4, 0.07)];
        let c = compute_confusion(&b);
        assert!(!c.is_confused);
    }

    #[test]
    fn market_fields_none_without_odds() {
        let b = vec![hp(1, 0.40), hp(2, 0.30)];
        let horses = build_board_horses(&b, &b, None, None);
        assert!(horses.iter().all(|h| h.market_implied.is_none()));
        assert!(horses.iter().all(|h| h.popularity.is_none()));
        // オッズ無しでも確率と model_rank は出る。
        assert_eq!(
            horses.iter().find(|h| h.horse_num == 1).unwrap().model_rank,
            1
        );
    }
}
