use std::collections::HashMap;

use paddock_domain::{
    EstimationConfig, HorseEntry, HorseExplanation, HorseFactors, HorseName, HorseProbability,
    JockeyName, RaceId, RaceOdds, TrackCondition, TrainerName,
};

use crate::error::{Error, Result};
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{OddsRepository, RaceCardRepository, StatsRepository};

use super::explain::build_explanation;
use super::features::{
    HorseSignals, RaceContext, TREND_WEIGHTS, build_factors, field_mean_weight,
    recent_form_from_runs, resolve_shared_factors, running_style_from_runs,
};

/// `predict_race_views` の戻り値（#272 確率分離）。同一レース・同一盤面順の 2 系統の確率と任意の根拠。
///
/// 循環断ち（EV=P_blended×odds の循環）のため、順位付け（軸/相手）と EV 計算で別系統の確率を使う:
/// - `blended`（市場ブレンド α=blend_alpha）= 軸/相手の順位付け用。市場情報を含み解像度が高い
///   （Phase A: 純モデルは本命をフラットにしか出せない）。
/// - `pure`（純モデル α=1.0・市場非依存）= EV=P_pure×odds の計算と「過去データ視点」表示用。
pub struct PredictionViews {
    pub blended: Vec<HorseProbability>,
    pub pure: Vec<HorseProbability>,
    /// 予想根拠（`with_explanation=false` なら空）。`pure`/`blended` と同じ盤面順。
    pub explanations: Vec<HorseExplanation>,
}

impl<R: StatsRepository + RaceCardRepository + OddsRepository, P: PdfParser, F: PdfFetcher>
    Interactor<R, P, F>
{
    /// 出馬表から各馬の win/place/show 確率を推定する。`blend_alpha = Some(α)` のとき、
    /// 当日の市場オッズ（単勝, `find_race_odds(.., None)` の最新スナップショット）の implied 確率と
    /// α（モデル重み）でブレンドする（#72）。`None` はモデルのみ（市場オッズを取得しない）。
    /// `track_condition = Some(..)` のとき、各馬の馬場状態別成績を factor に加える（#73）。
    /// 出馬表 PDF に馬場状態は無いため、呼び出し側が当日の値を渡す（未確定なら `None`）。
    pub async fn predict_race(
        &self,
        race_id: &RaceId,
        blend_alpha: Option<f64>,
        track_condition: Option<TrackCondition>,
    ) -> Result<Vec<HorseProbability>> {
        // with_explanation=false: 通常経路では根拠を組まず無駄な String 割当てを避ける（#274 レビュー）。
        // config は本番設定を 1 度だけ生成し factor 収集と推定で共有する（単一情報源, #274 レビュー）。
        let config = EstimationConfig::production();
        let (entry_factors, _) = self
            .collect_race_factors(race_id, track_condition, false, &config)
            .await?;
        self.estimate_and_blend(&entry_factors, race_id, blend_alpha, &config)
            .await
    }

    /// 過去データ視点（純モデル α=1.0）と市場ブレンド視点（α=`blend_alpha`）の両確率を、factor 収集
    /// 1 回・市場オッズ 1 回 fetch で返す（#272 確率分離）。順位付けには `blended`（解像度が高い）、
    /// EV 計算には `pure`（市場と独立＝循環断ち）を使う想定で、呼び出し側が `build_portfolio` に
    /// `rank=blended` / `ev=pure` を渡す。`with_explanation=true` のとき根拠も返す（`pure` と同じ盤面順）。
    ///
    /// `blended` は `estimate_and_blend(.., blend_alpha, ..)`（α<1.0 で市場 odds を 1 回 fetch）、
    /// `pure` は同 `Some(1.0)`（ブレンド無効で odds fetch を省く・γ 冪は両者に適用）。`blend_alpha=Some(1.0)`
    /// や `None` を渡すと両者が一致しうる（呼び出し側の選択に委ねる）。
    pub async fn predict_race_views(
        &self,
        race_id: &RaceId,
        blend_alpha: Option<f64>,
        track_condition: Option<TrackCondition>,
        with_explanation: bool,
    ) -> Result<PredictionViews> {
        // 本番フロー（session/predict-watch/recommend）は常に本番既定 config。m/γ 上書きは
        // predict_race_views_with_config 経由（analyze predict の #282 フラグ）でのみ効かせる。
        self.predict_race_views_with_config(
            race_id,
            blend_alpha,
            track_condition,
            with_explanation,
            &EstimationConfig::production(),
        )
        .await
    }

    /// `predict_race_views` の config 明示版（#282）。確率推定 config を呼び出し側が与える。
    /// analyze predict の `--shrinkage-m` / `--win-power` で本番既定 m=10 / γ=1.25 を上書きした
    /// bt_pred を生成するために使う（#270/ADR 0045 の m×α×γ 再検証）。本番フローは
    /// `predict_race_views`（config=production 固定）を使い、この経路の影響を受けない。
    async fn predict_race_views_with_config(
        &self,
        race_id: &RaceId,
        blend_alpha: Option<f64>,
        track_condition: Option<TrackCondition>,
        with_explanation: bool,
        config: &EstimationConfig,
    ) -> Result<PredictionViews> {
        let (entry_factors, explanations) = self
            .collect_race_factors(race_id, track_condition, with_explanation, config)
            .await?;
        let blended = self
            .estimate_and_blend(&entry_factors, race_id, blend_alpha, config)
            .await?;
        let pure = self
            .estimate_and_blend(&entry_factors, race_id, Some(1.0), config)
            .await?;
        debug_assert_eq!(
            blended.len(),
            pure.len(),
            "blended と pure の頭数が一致しない"
        );
        // with_explanation=true のとき根拠は確率と同じ盤面順・同頭数で揃う（collect_race_factors と
        // estimate_* が card.entries 順を保つ）。false のとき explanations は空。
        debug_assert!(
            !with_explanation
                || (explanations.len() == pure.len()
                    && explanations
                        .iter()
                        .zip(&pure)
                        .all(|(e, p)| e.horse_num == p.horse_num)),
            "explanations が pure と頭数/馬番でズレている"
        );
        Ok(PredictionViews {
            blended,
            pure,
            explanations,
        })
    }

    /// 出馬表の取得と各馬の factor / 予想根拠の構築までを共通化する内部ヘルパ（#274）。
    /// `predict_race`（確率のみ）と `predict_race_views`（確率＋根拠）が共有し、DB の二重取得を
    /// 避ける。返す `entry_factors` と `explanations` は同じ出走馬順で揃う。
    /// `with_explanation=false` のとき `explanations` は空 Vec を返す（通常経路で根拠を組まず
    /// 無駄な String 割当てを避ける。`predict_race` は確率しか使わないため）。
    /// `config` は呼び出し側が生成して渡す（推定と同じ設定を共有するため, #274 レビュー）。
    async fn collect_race_factors(
        &self,
        race_id: &RaceId,
        track_condition: Option<TrackCondition>,
        with_explanation: bool,
        config: &EstimationConfig,
    ) -> Result<(Vec<(HorseEntry, HorseFactors)>, Vec<HorseExplanation>)> {
        let card = self
            .repository
            .find_race_card(race_id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("race card: {}", race_id.value())))?;

        // コース統計は全馬共通なのでループ外で 1 回だけ取得する
        let course = self
            .repository
            .course_stats(card.venue, card.distance, card.surface, None)
            .await?;

        // 斤量のレース内相対シグナル用の field 平均斤量（#135）。斤量を持つ出走馬のみで平均する。
        // netkeiba 出馬表は斤量あり、PDF 出馬表は全馬 None なので平均も None（斤量項なし）。
        let mean_weight = field_mean_weight(card.entries.iter().filter_map(|e| e.weight_carried));
        let race_ctx = RaceContext {
            venue: card.venue,
            surface: card.surface,
            distance: card.distance,
            track_condition,
            field_size: card.entries.len(),
            mean_weight,
        };
        // 条件依存枠バイアス（#343・提示専用）。根拠生成時だけ 1 回取得する（18 セル集計。確率のみの
        // predict_race・backtest 経路では取得せずスコアにも影響させない＝measure-first）。
        let conditional_gate = if with_explanation {
            Some(
                self.repository
                    .conditional_gate_stats(card.venue, card.distance, card.surface, None)
                    .await?,
            )
        } else {
            None
        };
        // 確率推定設定（#75: ベイズ縮約 m=10。recency は production() で None）は呼び出し側から共有。
        // 前走タイムの相対速度シグナル用の標準タイム表（#76）。全馬共通なのでループ外で 1 回だけ
        // 取得する。cutoff=card.date で出馬表日以降をリークさせない。
        let standard_times = self.repository.standard_times(card.date).await?;

        // 全馬・騎手・調教師の名前を収集して 4 クエリで一括取得する（per-horse N+1 解消 #205）。
        // 重複排除（同一騎手が複数馬に騎乗する場合等）は各 _batch 実装の内部で行うため、
        // 呼び出し側は重複ありで渡してよい。
        let horse_names: Vec<HorseName> =
            card.entries.iter().map(|e| e.horse_name.clone()).collect();
        let jockey_names: Vec<JockeyName> = card
            .entries
            .iter()
            .filter_map(|e| e.jockey.clone())
            .collect();
        let trainer_names: Vec<TrainerName> = card
            .entries
            .iter()
            .filter_map(|e| e.trainer.clone())
            .collect();
        // as_of: None = 全期間統計（predict は出馬表日時点での履歴制限なし。
        // リーク防止の as_of は backtest 経路のみ必要）。
        // try_join! の実際の並列度は接続プールのコネクション数に依存する。
        let (horse_map, jockey_map, trainer_map, runs_map, jockey_form_map) = tokio::try_join!(
            self.repository.horse_stats_batch(&horse_names, None),
            self.repository.jockey_stats_batch(&jockey_names, None),
            self.repository.trainer_stats_batch(&trainer_names, None),
            // limit: TREND_WEIGHTS の要素数まで取得し、trend_n で何走使うかを scoring 側で制御する（#220）。
            self.repository
                .recent_runs_batch(&horse_names, card.date, TREND_WEIGHTS.len() as u32),
            self.repository.jockey_recent_runs_batch(
                &jockey_names,
                card.date,
                super::super::JOCKEY_RECENT_FORM_LIMIT,
            ),
        )?;

        let mut entry_factors: Vec<(HorseEntry, HorseFactors)> = Vec::new();
        let mut explanations: Vec<HorseExplanation> = Vec::new();
        for entry in &card.entries {
            // ok_or_else: batch 契約上 None になることはないが、rdb-gateway の override
            // バグを panic ではなく error として伝播させるため backtest の expect とは意図的に非対称。
            let horse = horse_map.get(&entry.horse_name).ok_or_else(|| {
                Error::NotFound(format!("horse stats: {}", entry.horse_name.value()))
            })?;
            // production() は recency: None なので horse_recency は取得しない（#75）。
            // jockey/trainer は DB 未登録（新人騎手・調教師交代等）が正当なケースのため horse と
            // 異なり ok_or_else でエラーにせず None とし母数から除外する（ADR 0007）。
            let jockey = entry.jockey.as_ref().and_then(|j| jockey_map.get(j));
            let trainer = entry.trainer.as_ref().and_then(|t| trainer_map.get(t));
            // 初戦馬（前走なし）は有効なケースのため unwrap_or(&[]) で空スライスを返す
            // （horse_map の ok_or_else との非対称は意図的）。
            let recent_runs = runs_map
                .get(&entry.horse_name)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let recent_form =
                recent_form_from_runs(recent_runs, card.date, &standard_times, config.trend_n);
            let jockey_recent_form = entry
                .jockey
                .as_ref()
                .and_then(|j| jockey_form_map.get(j))
                .and_then(|runs| paddock_domain::jockey_recent_form_score(runs));
            // 脚質（先行度, #329 Phase1）。近走のコーナー通過順位＋頭数から算出（本番は重み 0 で挙動不変）。
            let running_style = running_style_from_runs(recent_runs);
            // build_factors と build_explanation が共有する条件別成績を 1 回だけ解決する（#409）。
            let shared = resolve_shared_factors(entry, &course, horse, jockey, trainer, &race_ctx);
            let signals = HorseSignals {
                recent_form,
                jockey_recent_form,
                running_style,
                // 斤量のレース内相対シグナル（#135）。当該馬の斤量と field 平均斤量の両方があるときのみ項を立てる。
                // PDF 出馬表（斤量なし）・field 平均が出せないレースは None（母数除外）。
                weight_carried: entry
                    .weight_carried
                    .zip(race_ctx.mean_weight)
                    .map(|(w, mean)| paddock_domain::prediction::weight_factor(w, mean)),
            };
            let factors = build_factors(
                &shared, &race_ctx, &signals, None, // recency: production() では無効
                card.date, config,
            );
            // 予想根拠（#274）。確率推定と同じ共有 factor レート・前走から作る。runs は date 降順なので
            // index 0 が最新（前走）。本番経路は recency: None なので集計レート（build_factors と同源）。
            // with_explanation=false（通常の predict_race）では組まずに無駄な String 割当てを避ける。
            if with_explanation {
                explanations.push(build_explanation(
                    &shared,
                    entry,
                    conditional_gate.as_ref(),
                    &race_ctx,
                    recent_form,
                    recent_runs.first(),
                ));
            }
            entry_factors.push((entry.clone(), factors));
        }

        Ok((entry_factors, explanations))
    }

    /// 構築済みの `entry_factors` から確率を推定し、市場オッズブレンド・冪変換まで適用する（#72/#246）。
    /// `predict_race` と `predict_race_views` が共有する（後者は α を変えて 2 回呼び blended/pure を得る）。
    /// `config` は呼び出し側が生成し `collect_race_factors` と同じ設定を渡す（単一情報源, #274 レビュー）。
    async fn estimate_and_blend(
        &self,
        entry_factors: &[(HorseEntry, HorseFactors)],
        race_id: &RaceId,
        blend_alpha: Option<f64>,
        config: &EstimationConfig,
    ) -> Result<Vec<HorseProbability>> {
        // estimate_probabilities が win→1.0 / place→2.0 / show→3.0 正規化 + 累積 max 単調化を行い、
        // win_prob ≤ place_prob ≤ show_prob を保証する（ADR 0007）。本番経路は #75 で採用した
        // ベイズ縮約（m=10）を有効にし、少データ馬の過信（win_prob=0 を含む）を緩和する。
        let probs =
            paddock_domain::prediction::estimate_probabilities_with_config(entry_factors, config);

        // 市場オッズ（単勝）ブレンド（#72）。α<1.0 のときのみ最新オッズスナップショットを取得する
        // （α>=1.0・非有限はブレンド無効なので DB クエリを省く）。
        let probs = match blend_alpha.filter(|a| a.is_finite() && *a < 1.0) {
            Some(alpha) => {
                let market = self.repository.find_race_odds(race_id, None).await?;
                match market {
                    Some(odds) => {
                        let market_win: HashMap<_, _> =
                            odds.win.iter().map(|(num, o)| (*num, o.value())).collect();
                        paddock_domain::prediction::blend_with_market_win(
                            &probs,
                            &market_win,
                            alpha,
                        )
                    }
                    None => probs,
                }
            }
            None => probs,
        };

        // 穴馬の 1 着過大評価を縮約する win_prob 冪変換（#246）。config.win_power が None なら no-op。
        // ブレンド後の最終 win に適用し、連系・着順 EV（Harville/simulate）まで校正後 win が伝わる。
        let probs = match config.win_power {
            Some(gamma) => paddock_domain::prediction::apply_win_power(&probs, gamma),
            None => probs,
        };

        Ok(probs)
    }

    /// `predict_race_views` に加え、最新オッズスナップショット（`find_race_odds(.., None)`）も返す。
    /// analyze predict 等、二視点（過去データ視点=`pure`・市場EV視点=`blended`×odds）を自前で
    /// 組み立てる呼び出し側向け（#272 ③④）。オッズ未取得なら `None`（過去データ視点だけ出せる）。
    /// 診断は呼び出し側が `pair_ev_diagnostics(&blended, &pure, &odds, ..)` を実行する（軸=`blended`・
    /// EV=`pure` の循環断ち #272。session.rs と同じ経路）。オッズは保存スナップショット参照。
    /// `config` で確率推定の m/γ を上書きできる（#282。本番は `EstimationConfig::production()` を渡す）。
    pub async fn predict_race_views_with_odds(
        &self,
        race_id: &RaceId,
        blend_alpha: Option<f64>,
        track_condition: Option<TrackCondition>,
        with_explanation: bool,
        config: &EstimationConfig,
    ) -> Result<(PredictionViews, Option<RaceOdds>)> {
        let views = self
            .predict_race_views_with_config(
                race_id,
                blend_alpha,
                track_condition,
                with_explanation,
                config,
            )
            .await?;
        let odds = self.repository.find_race_odds(race_id, None).await?;
        Ok((views, odds))
    }
}
