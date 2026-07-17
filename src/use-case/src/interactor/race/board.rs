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
    HorseExplanation, HorseNum, HorseProbability, KONSEN_BAND_RATIO, KONSEN_MIN_HORSES, Mark,
    PadPrediction, Portfolio, PortfolioConfig, RaceId, RaceOdds, TrackCondition, build_portfolio,
    konsen_band,
};

use crate::error::Result;
use crate::interactor::Interactor;
use crate::interactor::race::commentary::{horse_detail_lines, horse_headline, race_commentary};
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{
    OddsRepository, PadPredictionRepository, RaceCardRepository, RaceResultRepository,
    StatsRepository,
};

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
    /// 表示用レース名（#389。例「七夕賞」。netkeiba 経路のみ、無ければ `None`）。
    pub race_name: Option<String>,
    /// 格付けスラッグ（#345。例 `g3` / `open`。無ければ `None`）。盤ヘッダのグレード表記に使う。
    pub race_class: Option<String>,
    /// 買い目ポートフォリオ（保存オッズが無ければ `None`＝買い目は組めない。盤は確率だけで描ける）。
    /// 軸は記録軸（`recorded_axis`）があればそれに固定される（#388・軸ロック）。
    pub portfolio: Option<Portfolio>,
    /// predict 記録済みの本命◎（軸ロックの正）。pad の Honmei 印がその馬が出走している場合のみ。
    /// 未 predict・取消時は `None`（＝買い目軸はライブ再計算 `live_axis` にフォールバック, #388）。
    pub recorded_axis: Option<u32>,
    /// ライブ再計算の軸＝blended（市場ブレンド α=0.2）首位（機械◎, `model_rank==1`）。
    /// 記録軸との乖離警告に使う。直前オッズで動くのはこちら（#388）。
    pub live_axis: Option<u32>,
    pub confusion: Confusion,
    /// レース書評（混戦度・◎の狙いどころ・妙味）。人手 `PadPrediction.commentary` があれば優先、
    /// 無ければルールベース生成（#348）。◎が無い等で生成できなければ `None`。
    pub race_comment: Option<String>,
    /// 結果確定フラグ（#381。`results` に着順ありの行が 1 件以上）。web の「⚫終」判定・着順表示に使う。
    pub result_confirmed: bool,
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
    /// EV 視点（純モデル α=1.0・市場非依存）の勝率 [0,1]。連対/複勝は下記（#373 盤の3系統表示）。
    pub pure_win_prob: f64,
    /// 純モデル α=1.0 の連対率/複勝率 [0,1]（#373）。
    pub pure_place_prob: f64,
    pub pure_show_prob: f64,
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
    /// 確定着順（#381。`results` 由来。未確定・除外/中止で着順なしは `None`）。
    pub finishing_position: Option<u32>,
    /// 馬書評の一行寸評（headline）。人手 `PredictionHorse.comment` があれば優先、無ければ
    /// ルールベース生成（#348）。特筆材料が無ければ `None`。
    pub comment: Option<String>,
    /// 展開パネル用の根拠 bullet（条件別 factor・枠 lift・近走・前走・斤量）。空なら根拠情報なし。
    pub detail_lines: Vec<String>,
}

impl<
    R: StatsRepository
        + RaceCardRepository
        + OddsRepository
        + PadPredictionRepository
        + RaceResultRepository,
    P: PdfParser,
    F: PdfFetcher,
> Interactor<R, P, F>
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
        // 書評レイヤ（#348）のため with_explanation=true で根拠を取得する（盤は on-demand で
        // hot loop 非経由のため conditional_gate_stats 1 回追加の負荷は許容）。
        let views = self
            .predict_race_views(race_id, blend_alpha, track_condition, true)
            .await?;
        let card = self.race_card(race_id).await?;
        let odds = self.repository.find_race_odds(race_id, None).await?;

        // 人手予想（印・短評）があれば overlay の材料に取得する（未保存なら None）。
        let pad = match card.as_ref() {
            Some(c) => {
                self.repository
                    .find_pad_prediction(c.date, c.venue, c.race_num)
                    .await?
            }
            None => None,
        };

        // 軸ロック（#388）: 記録軸＝pad の◎、ライブ再計算軸＝blended 首位。純関数に切り出し単体テスト可能に。
        let recorded_axis = recorded_axis_of(pad.as_ref(), &views.blended);
        let live_axis = live_axis_of(&views.blended);

        // 買い目は既存経路（相手 top5 不変）。軸は記録軸があればそれに固定（#388）。オッズ無しなら組まない。
        let forced_axis = recorded_axis.and_then(|n| HorseNum::try_from(n).ok());
        let portfolio = odds.as_ref().map(|o| {
            build_portfolio(
                &views.blended,
                &views.pure,
                o,
                budget,
                &PortfolioConfig {
                    forced_axis,
                    ..PortfolioConfig::default()
                },
            )
        });

        let mut horses =
            build_board_horses(&views.blended, &views.pure, odds.as_ref(), card.as_ref());
        // 確定着順（#381）を results から後付けする。着順ありの行が 1 件でもあれば結果確定。
        let finishing = self.repository.find_finishing_positions(race_id).await?;
        let result_confirmed = !finishing.is_empty();
        for h in horses.iter_mut() {
            h.finishing_position = finishing.get(&h.horse_num).copied();
        }
        let confusion = compute_confusion(&views.blended);
        enrich_commentary(&mut horses, &views.explanations, pad.as_ref());

        // レース書評: 人手 commentary 優先、無ければルールベース生成（◎不在なら None）。
        let race_comment = resolve_race_comment(
            pad.as_ref().and_then(|p| p.commentary.as_deref()),
            &confusion,
            &horses,
        );

        let (venue, surface, distance, race_num, date, post_time, race_name, race_class) =
            match card.as_ref() {
                Some(c) => (
                    c.venue.as_slug().to_string(),
                    c.surface.as_str().to_string(),
                    c.distance,
                    c.race_num,
                    c.date,
                    c.post_time.map(|t| t.format("%H:%M").to_string()),
                    c.race_name.clone(),
                    c.race_class.map(|rc| rc.as_str().to_string()),
                ),
                // 出馬表が無ければ predict_race_views で NotFound になるのでここには基本到達しない。
                None => (
                    String::new(),
                    String::new(),
                    0,
                    0,
                    NaiveDate::default(),
                    None,
                    None,
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
            race_name,
            race_class,
            portfolio,
            recorded_axis,
            live_axis,
            confusion,
            race_comment,
            result_confirmed,
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
            // pure と blended は同一 predict_race_views 由来で常に同じ馬集合。見つからないのは
            // 到達しないが、防御的に blended 値へフォールバックする（純モデル列に無警告で混入しない
            // よう、本来は同集合であることを前提とする）。
            let pure_hp = pure.iter().find(|p| p.horse_num == hp.horse_num);
            let pure_win_prob = pure_hp.map(|p| p.win_prob).unwrap_or(hp.win_prob);
            let pure_place_prob = pure_hp.map(|p| p.place_prob).unwrap_or(hp.place_prob);
            let pure_show_prob = pure_hp.map(|p| p.show_prob).unwrap_or(hp.show_prob);
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
                pure_place_prob,
                pure_show_prob,
                market_implied: implied.get(&num).copied(),
                win_odds,
                place_odds_low: place.map(|p| p.low.value()),
                place_odds_high: place.map(|p| p.high.value()),
                popularity: pop,
                model_rank: mr,
                mark,
                is_overlay,
                is_value,
                // 着順は race_board で results から後付けする（build_board_horses は IO 非依存の純関数に保つ）。
                finishing_position: None,
                // 書評は enrich_commentary で後付けする（build_board_horses は IO 非依存の純関数に保つ）。
                comment: None,
                detail_lines: Vec::new(),
            }
        })
        .collect()
}

/// 盤行に書評（一行寸評＋根拠 bullet）を後付けする（#348）。`explanations` は
/// `with_explanation=true` の predict 由来（馬番で突き合わせ）。一行寸評は人手 `pad` の短評を
/// ルールベース生成より優先する（overlay）。`explanations` が空でも人手 comment の overlay は効く
/// （その場合 detail_lines は空・ルールベース headline は付かない）。空文字の人手短評は採用しない。
fn enrich_commentary(
    horses: &mut [BoardHorse],
    explanations: &[HorseExplanation],
    pad: Option<&PadPrediction>,
) {
    for h in horses.iter_mut() {
        let expl = explanations
            .iter()
            .find(|e| e.horse_num.value() == h.horse_num);
        // 一行寸評: 人手コメント優先（空文字は不採用）、無ければ factor/前走/近走のルールベース。
        let human = pad
            .and_then(|p| {
                p.horses
                    .iter()
                    .find(|ph| ph.horse_num == h.horse_num)
                    .and_then(|ph| ph.comment.clone())
            })
            .filter(|s| !s.trim().is_empty());
        h.comment = human.or_else(|| expl.and_then(horse_headline));
        h.detail_lines = expl.map(horse_detail_lines).unwrap_or_default();
    }
}

/// レース書評を決める（#348）。人手 `human` が非空ならそれを優先、無ければルールベース生成を返す。
/// ルールベースが空（◎不在）なら `None`。人手優先の分岐を純関数に切り出して単体テスト可能にする。
fn resolve_race_comment(
    human: Option<&str>,
    confusion: &Confusion,
    horses: &[BoardHorse],
) -> Option<String> {
    if let Some(h) = human.filter(|h| !h.trim().is_empty()) {
        return Some(h.to_string());
    }
    let generated = race_commentary(confusion, horses);
    (!generated.is_empty()).then_some(generated)
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

/// 記録軸（#388 軸ロック）: `pad` の◎(Honmei) 馬番。ただし出走集合 `blended` に在るときだけ有効に
/// する（取消等で非出走なら失効＝ライブ再計算へフォールバック）。未 predict・◎不在は `None`。
fn recorded_axis_of(pad: Option<&PadPrediction>, blended: &[HorseProbability]) -> Option<u32> {
    pad.and_then(|p| {
        // ◎は人手予想で 1 頭前提。異常データで複数あっても記録（Vec）順の先頭を採る。
        p.horses
            .iter()
            .find(|h| h.mark == Some(Mark::Honmei))
            .map(|h| h.horse_num)
            .filter(|&n| blended.iter().any(|hp| hp.horse_num.value() == n))
    })
}

/// ライブ再計算軸（#388）: `blended`（市場ブレンド α=0.2）首位＝機械◎（`model_rank==1`）。
/// 記録軸との乖離警告に使う（直前オッズで動くのはこちら）。空なら `None`。
fn live_axis_of(blended: &[HorseProbability]) -> Option<u32> {
    model_ranks(blended)
        .into_iter()
        .find(|(_, rank)| *rank == 1)
        .map(|(num, _)| num)
}

/// 混戦判定（CLAUDE.md）: ◎（勝率1位）の勝率 × 0.70 以上の馬が ◎含め 4 頭以上。
/// 判定ロジックは domain の [`konsen_band`]（`build_portfolio` の混戦分岐と同一の真実源）を再利用し、
/// 盤面表示に要る `axis_win_prob` / `threshold`（母集団しきい値）だけをここで補う。
fn compute_confusion(blended: &[HorseProbability]) -> Confusion {
    let axis_win_prob = blended.iter().map(|hp| hp.win_prob).fold(0.0_f64, f64::max);
    let threshold = axis_win_prob * KONSEN_BAND_RATIO;
    let band = konsen_band(blended);
    Confusion {
        is_confused: axis_win_prob > 0.0 && band.len() >= KONSEN_MIN_HORSES,
        axis_win_prob,
        threshold,
        qualifying_count: band.len() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paddock_domain::{
        ExplainCategory, FactorExplanation, HorseName, HorseNum, PredictionHorse, RateTriple, Venue,
    };

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
    fn pure_place_show_come_from_pure_not_blended() {
        // #373: 純モデルの連対率/複勝率は pure 由来（表示の勝/連/複＝ブレンドとは別系統）。
        // hp: place=win*2, show=(win*3).min(1.0)。blended=0.40→place 0.80/show 1.0(クランプ)、
        // pure=0.20→place 0.40/show 0.60。表示系(ブ)と純系(モ)が別値になる組で弁別する。
        let blended = vec![hp(1, 0.40)];
        let pure = vec![hp(1, 0.20)];
        let horses = build_board_horses(&blended, &pure, None, None);
        let h = &horses[0];
        assert!((h.win_prob - 0.40).abs() < 1e-9, "表示勝率はブレンド");
        assert!((h.place_prob - 0.80).abs() < 1e-9, "表示連対率はブレンド");
        assert!((h.pure_win_prob - 0.20).abs() < 1e-9, "モ勝は純モデル");
        assert!((h.pure_place_prob - 0.40).abs() < 1e-9, "モ連は純モデル");
        assert!((h.pure_show_prob - 0.60).abs() < 1e-9, "モ複は純モデル");
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

    /// ◎(Honmei) を `honmei` に置いた pad（#388 テスト用）。
    fn pad_axis(honmei: u32) -> PadPrediction {
        PadPrediction {
            date: NaiveDate::default(),
            venue: Venue::Hakodate,
            race_num: 1,
            title: None,
            budget: None,
            strategy_note: None,
            commentary: None,
            horses: vec![PredictionHorse {
                horse_num: honmei,
                horse_name: format!("馬{honmei}"),
                jockey: None,
                mark: Some(Mark::Honmei),
                win_odds: None,
                popularity: None,
                win_prob: None,
                place_prob: None,
                show_prob: None,
                comment: None,
            }],
            bets: vec![],
            result: None,
        }
    }

    #[test]
    fn recorded_axis_reads_honmei_in_field() {
        // ◎=8 が出走集合に居る → 記録軸=8（blended 首位 5 とは別＝ロックが効く条件）。
        let blended = vec![hp(5, 0.30), hp(8, 0.25), hp(1, 0.10)];
        assert_eq!(recorded_axis_of(Some(&pad_axis(8)), &blended), Some(8));
    }

    #[test]
    fn recorded_axis_none_when_honmei_scratched() {
        // ◎=8 が非出走（取消でblendedに不在）→ 失効して None（ライブ再計算へフォールバック）。
        let blended = vec![hp(5, 0.30), hp(1, 0.10)];
        assert_eq!(recorded_axis_of(Some(&pad_axis(8)), &blended), None);
    }

    #[test]
    fn recorded_axis_none_without_pad_or_mark() {
        let blended = vec![hp(5, 0.30)];
        assert_eq!(recorded_axis_of(None, &blended), None);
        // pad はあるが◎印なし（pad_with は mark=None）→ None。
        assert_eq!(
            recorded_axis_of(Some(&pad_with(5, None, None)), &blended),
            None
        );
    }

    #[test]
    fn live_axis_is_blended_top() {
        // 機械◎(model_rank==1)＝blended 首位。同率は馬番昇順。
        let blended = vec![hp(5, 0.30), hp(8, 0.25), hp(1, 0.10)];
        assert_eq!(live_axis_of(&blended), Some(5));
        assert_eq!(live_axis_of(&[]), None);
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

    /// Surface の Strong factor 1 本を持つ説明（headline "芝が得意。"・detail 1 行を生む）。
    fn expl_surface_strong(num: u32) -> HorseExplanation {
        HorseExplanation {
            horse_num: HorseNum::try_from(num).unwrap(),
            horse_name: HorseName::try_from(format!("馬{num}")).unwrap(),
            factors: vec![FactorExplanation::new(
                ExplainCategory::Surface,
                "芝".to_string(),
                RateTriple {
                    win: 0.3,
                    place: 0.5,
                    show: 0.7,
                },
                20,
            )],
            recent_form: None,
            prev_run: None,
            gate_bias_lift: None,
            weight_carried: None,
            field_mean_weight: None,
        }
    }

    fn pad_with(num: u32, comment: Option<&str>, commentary: Option<&str>) -> PadPrediction {
        PadPrediction {
            date: NaiveDate::default(),
            venue: Venue::Hakodate,
            race_num: 1,
            title: None,
            budget: None,
            strategy_note: None,
            commentary: commentary.map(str::to_string),
            horses: vec![PredictionHorse {
                horse_num: num,
                horse_name: format!("馬{num}"),
                jockey: None,
                mark: None,
                win_odds: None,
                popularity: None,
                win_prob: None,
                place_prob: None,
                show_prob: None,
                comment: comment.map(str::to_string),
            }],
            bets: vec![],
            result: None,
        }
    }

    #[test]
    fn enrich_prefers_human_comment_over_rulebase() {
        let b = vec![hp(1, 0.4)];
        let mut horses = build_board_horses(&b, &b, None, None);
        let expls = vec![expl_surface_strong(1)];
        let pad = pad_with(1, Some("人手の寸評"), None);
        enrich_commentary(&mut horses, &expls, Some(&pad));
        assert_eq!(horses[0].comment.as_deref(), Some("人手の寸評"));
        // detail_lines は explanation 由来（人手 comment とは独立に付く）。
        assert!(!horses[0].detail_lines.is_empty());
    }

    #[test]
    fn enrich_falls_back_to_rulebase_when_no_human() {
        let b = vec![hp(1, 0.4)];
        let mut horses = build_board_horses(&b, &b, None, None);
        let expls = vec![expl_surface_strong(1)];
        enrich_commentary(&mut horses, &expls, None);
        assert_eq!(horses[0].comment.as_deref(), Some("芝が得意。"));
    }

    #[test]
    fn enrich_ignores_blank_human_comment() {
        let b = vec![hp(1, 0.4)];
        let mut horses = build_board_horses(&b, &b, None, None);
        let expls = vec![expl_surface_strong(1)];
        let pad = pad_with(1, Some("   "), None); // 空白のみ → 不採用
        enrich_commentary(&mut horses, &expls, Some(&pad));
        assert_eq!(
            horses[0].comment.as_deref(),
            Some("芝が得意。"),
            "空白のみの人手はルールベースにフォールバック"
        );
    }

    #[test]
    fn enrich_human_overlay_applies_without_explanation() {
        // explanations 空でも人手 comment の overlay は効く（detail_lines は空）。
        let b = vec![hp(1, 0.4)];
        let mut horses = build_board_horses(&b, &b, None, None);
        let pad = pad_with(1, Some("人手のみ"), None);
        enrich_commentary(&mut horses, &[], Some(&pad));
        assert_eq!(horses[0].comment.as_deref(), Some("人手のみ"));
        assert!(horses[0].detail_lines.is_empty());
    }

    #[test]
    fn resolve_race_comment_prefers_human() {
        let b = vec![hp(1, 0.4), hp(2, 0.1)];
        let horses = build_board_horses(&b, &b, None, None);
        let c = compute_confusion(&b);
        assert_eq!(
            resolve_race_comment(Some("人手レース評"), &c, &horses).as_deref(),
            Some("人手レース評")
        );
    }

    #[test]
    fn resolve_race_comment_generates_or_blank_falls_back() {
        let b = vec![hp(1, 0.4), hp(2, 0.1)];
        let horses = build_board_horses(&b, &b, None, None);
        let c = compute_confusion(&b);
        // 人手なし → ルールベース。空白のみの人手も同じくルールベースへ。
        assert!(
            resolve_race_comment(None, &c, &horses)
                .unwrap()
                .contains("◎馬1")
        );
        assert!(
            resolve_race_comment(Some("  "), &c, &horses)
                .unwrap()
                .contains("◎馬1")
        );
    }

    #[test]
    fn resolve_race_comment_none_without_axis() {
        // ◎（model_rank==1）不在＝全頭空ならルールベースも空 → None。
        let c = compute_confusion(&[]);
        assert_eq!(resolve_race_comment(None, &c, &[]), None);
    }
}
