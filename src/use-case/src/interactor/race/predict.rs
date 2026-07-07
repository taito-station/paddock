use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{
    EstimationConfig, ExplainCategory, FactorExplanation, FactorStat, HorseEntry, HorseExplanation,
    HorseFactors, HorseName, HorseProbability, JockeyName, PrevRunSummary, RaceId, RaceOdds,
    RateTriple, RecentRun, StandardTimes, Surface, TrackCondition, TrainerName,
};

use crate::error::{Error, Result};
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{
    ConditionalGateStatsRow, CourseStatsRow, GroupStat, HorseRecencyStats, HorseStatsRow,
    JockeyStatsRow, OddsRepository, RaceCardRepository, RecencySeries, StatsRepository,
    TrainerStatsRow, gate_field_band_label, gate_track_cond2_label,
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
                super::JOCKEY_RECENT_FORM_LIMIT,
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
            let factors = build_factors(
                entry,
                &course,
                horse,
                jockey,
                trainer,
                &race_ctx,
                recent_form,
                None, // recency: production() では無効
                card.date,
                config,
                jockey_recent_form,
                running_style,
            );
            // 予想根拠（#274）。確率推定と同じ factor レート・前走から作る。runs は date 降順なので
            // index 0 が最新（前走）。本番経路は recency: None なので集計レート（build_factors と同源）。
            // with_explanation=false（通常の predict_race）では組まずに無駄な String 割当てを避ける。
            if with_explanation {
                explanations.push(build_explanation(
                    entry,
                    &course,
                    conditional_gate.as_ref(),
                    horse,
                    jockey,
                    trainer,
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

/// 直近 N 走トレンドの重み（#220）。runs は date 降順なので index 0 が最新走。
/// `[1.0, 0.5, 0.25]` = Issue #220 指定の指数的減衰ウェイト。
/// `pub(crate)` にして backtest.rs からも参照できるようにする（バッチ取得上限と一致させるため）。
/// この配列を変更する場合は ADR-0036・CLI の `--trend-n` help・仕様書（probability-estimation.md）も更新すること。
pub(crate) const TREND_WEIGHTS: [f64; 3] = [1.0, 0.5, 0.25];

/// 取得済みの近走 `runs`（date 降順、最大 `limit` 件）から前走フォーム [0,1] を算出する純粋関数。
/// `recent_form_for` の DB 取得を剥がした本体で、backtest のバッチ取得（#196）からも共有する。
///
/// `trend_n = 1` のとき直近 1 走スコアのみ返し、現行挙動と完全一致する。
/// `trend_n >= 2` のとき有効スコアが得られた走を TREND_WEIGHTS で加重平均する。
/// スコアが取れなかった走（中止・情報欠落等）は分母から除外（欠落フォールバック維持）。
///
/// `before` は予測対象レースの日付（cutoff）。`recent_form_score` の間隔シグナルは
/// cutoff と各走の日付の差で算出するため N 走すべてに同じ `before` を渡す
/// （走間の間隔ではなく cutoff 基準）。リーク防止（before 以降の走を除外）も兼ねる。
/// `runs` は呼び出し元が `date < before` でフィルタ済みであることを前提とする
/// （この関数内では再チェックしない）。
/// `trend_n` は 1 以上でなければならない（CLI バリデーション済み）。
pub(crate) fn recent_form_from_runs(
    runs: &[RecentRun],
    before: NaiveDate,
    standard_times: &StandardTimes,
    trend_n: u32,
) -> Option<f64> {
    debug_assert!(trend_n >= 1, "trend_n must be >= 1, got {trend_n}");
    // 三重 min: trend_n 上界 → 近走実在数 → TREND_WEIGHTS 配列長（CLI バリデーション済みだが防衛的に維持）。
    let n = (trend_n as usize).min(runs.len()).min(TREND_WEIGHTS.len());
    let mut wsum = 0.0_f64;
    let mut wden = 0.0_f64;
    for (i, run) in runs[..n].iter().enumerate() {
        let std = standard_times.get(run.surface, run.distance);
        if let Some(s) =
            paddock_domain::prediction::recent_form_score(&run.result, run.date, before, std)
        {
            wsum += TREND_WEIGHTS[i] * s;
            wden += TREND_WEIGHTS[i];
        }
    }
    (wden > 0.0).then(|| wsum / wden)
}

/// 取得済みの近走 `runs`（date 降順）から脚質（先行度）スカラー [0,1]（1=逃げ・0=追込）を算出する
/// 純粋関数（#329 Phase1）。各走のコーナー通過順位を頭数で相対化した先行度の単純平均。corner/頭数が
/// 取れない走（pdf 成績・未 backfill・中止等）は母数から除外し、有効走 0 は `None`（母数除外）。
/// 前走フォームと異なり trend 重みは掛けない（measure-first の最小形。効けば次段で重み付けを検討）。
/// `recent_form_from_runs` と同じく predict/backtest 両経路で共有する。
pub(crate) fn running_style_from_runs(runs: &[RecentRun]) -> Option<f64> {
    let mut sum = 0.0_f64;
    let mut n = 0u32;
    for run in runs {
        if let Some(v) = paddock_domain::prediction::running_style_of_run(
            run.corner_positions.as_deref(),
            run.field_size,
        ) {
            sum += v;
            n += 1;
        }
    }
    (n > 0).then(|| sum / n as f64)
}

/// `build_factors` に渡すレース側の条件（全馬共通）。
pub(crate) struct RaceContext {
    pub surface: Surface,
    pub distance: u32,
    /// 評価対象レースの馬場状態（#73）。未確定なら `None`（馬場項なし）。
    pub track_condition: Option<TrackCondition>,
    /// 出走頭数（#343 条件依存枠バイアスの頭数帯判定に使う。＝出馬表エントリ数）。
    pub field_size: usize,
    /// レース内の field 平均斤量[kg]（#135）。斤量を持つ出走馬が居ないレース（PDF 出馬表等）は
    /// `None`（斤量項なし）。`build_factors` が各馬の `weight_carried` との差から相対シグナルを作る。
    pub mean_weight: Option<f64>,
}

/// 取得済みの stats 行と前走フォームから `HorseFactors` を組み立てる純粋変換。本番 predict
/// （全期間統計）とバックテスト（as-of 統計）の両方から共有する。`recent_form` は呼び出し側が
/// 前走から算出して渡す（#31）。
///
/// `config.recency = Some(rc)` かつ `recency` が渡されたとき、馬自身の 3 factor（芝ダ・距離帯・
/// 馬場状態）は集計レートの代わりに日付付き系列を時間減衰した recency 重み付きレートで評価する
/// （#75 Phase B）。`as_of_date` は減衰の基準日（predict は出馬表日、backtest はレース日）。
/// course/jockey/trainer は従来の集計レートのまま。
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_factors(
    entry: &HorseEntry,
    course: &CourseStatsRow,
    horse: &HorseStatsRow,
    jockey: Option<&JockeyStatsRow>,
    trainer: Option<&TrainerStatsRow>,
    race: &RaceContext,
    recent_form: Option<f64>,
    recency: Option<&HorseRecencyStats>,
    as_of_date: NaiveDate,
    config: &EstimationConfig,
    jockey_recent_form: Option<f64>,
    running_style: Option<f64>,
) -> HorseFactors {
    let gate_label = gate_group_label(entry.gate_num.value());
    let surf_label = surface_label(race.surface);
    let dist_label = distance_band_label(race.distance);

    // recency 有効時は horse 系 factor を日付系列の時間減衰で評価する。無効時・系列なしは集計レート。
    let recency_cfg = config.recency.zip(recency);
    let horse_surface = match recency_cfg {
        Some((rc, r)) => recency_factor(&r.by_surface, surf_label, as_of_date, rc.half_life_days),
        None => stat_to_triple_opt(&horse.by_surface, surf_label),
    };
    let horse_distance = match recency_cfg {
        Some((rc, r)) => recency_factor(
            &r.by_distance_band,
            dist_label,
            as_of_date,
            rc.half_life_days,
        ),
        None => stat_to_triple_opt(&horse.by_distance_band, dist_label),
    };
    let horse_track_condition = race.track_condition.and_then(|tc| match recency_cfg {
        Some((rc, r)) => recency_factor(
            &r.by_track_condition,
            tc.as_str(),
            as_of_date,
            rc.half_life_days,
        ),
        None => stat_to_triple_opt(&horse.by_track_condition, tc.as_str()),
    });

    // 全 factor で「実績なし」を None（母数除外）に統一する（#81/ADR 0014）。一致なし・出走 0 件は
    // stat_to_triple_opt が None を返し、0 レート（＝全敗）と区別される。jockey/trainer は
    // 騎手・調教師欠落（and_then の外側 None）と「実績なし」（内側 None）を二段で畳む。
    HorseFactors {
        course_gate: stat_to_triple_opt(&course.by_gate_group, gate_label),
        horse_surface,
        horse_distance,
        jockey_surface: jockey.and_then(|j| stat_to_triple_opt(&j.by_surface, surf_label)),
        trainer_surface: trainer.and_then(|t| stat_to_triple_opt(&t.by_surface, surf_label)),
        // 馬場状態が未確定のレース・該当馬場での出走実績が無い馬は None（#73）。
        horse_track_condition,
        recent_form,
        // 斤量のレース内相対シグナル（#135）。当該馬の斤量と field 平均斤量の両方があるときのみ項を立てる。
        // PDF 出馬表（斤量なし）・field 平均が出せないレースは None（母数除外）。
        weight_carried: entry
            .weight_carried
            .zip(race.mean_weight)
            .map(|(w, mean)| paddock_domain::prediction::weight_factor(w, mean)),
        jockey_recent_form,
        running_style,
    }
}

/// 取得済みの stats 行・前走から各馬の予想根拠 [`HorseExplanation`] を組み立てる純粋変換（#274）。
/// `build_factors` と同じ素材（同一ラベルの集計レート・前走）を使うが、score への合成ではなく
/// 「人が読める条件別成績」へ写像する。本番経路は recency: None なので集計レート（build_factors と
/// 同源）を使い、確率推定と根拠の数値が一致する。`prev_run` は最新走（runs の index 0）。
///
/// **build_factors との同期が前提**: ラベル選択（`*_label`）と `stat_to_triple_opt` の組は
/// `build_factors` と揃えて「根拠の数値＝score の入力」を保つ。factor を増やす際は両方を更新する。
/// 確率自体は `predict_race` と `predict_race_views` が同一の `collect_race_factors`＋
/// `estimate_and_blend` を通るため構造的に一致する（`with_explanation` フラグは根拠生成の有無だけを
/// 切り替え、`entry_factors` には影響しない）。
/// 既知の乖離点: `config.recency = Some(..)` を有効化すると build_factors は時間減衰レート・本関数は
/// 集計レートになり数値がズレる。現 production は recency None のため実害なし（有効化時に追従が必要）。
#[allow(clippy::too_many_arguments)]
fn build_explanation(
    entry: &HorseEntry,
    course: &CourseStatsRow,
    conditional_gate: Option<&ConditionalGateStatsRow>,
    horse: &HorseStatsRow,
    jockey: Option<&JockeyStatsRow>,
    trainer: Option<&TrainerStatsRow>,
    race: &RaceContext,
    recent_form: Option<f64>,
    prev_run: Option<&RecentRun>,
) -> HorseExplanation {
    let gate_label = gate_group_label(entry.gate_num.value());
    let surf_label = surface_label(race.surface);
    let dist_label = distance_band_label(race.distance);

    let mut factors: Vec<FactorExplanation> = Vec::new();
    // 馬の条件別成績（芝ダ・距離帯）。一致なし・出走 0 件は stat_to_triple_opt が None（母数除外と同じ欠落扱い）。
    if let Some(fs) = stat_to_triple_opt(&horse.by_surface, surf_label) {
        factors.push(FactorExplanation::new(
            ExplainCategory::Surface,
            surf_label.to_string(),
            fs.rate,
            fs.starts,
        ));
    }
    if let Some(fs) = stat_to_triple_opt(&horse.by_distance_band, dist_label) {
        factors.push(FactorExplanation::new(
            ExplainCategory::Distance,
            dist_label.to_string(),
            fs.rate,
            fs.starts,
        ));
    }
    // 馬場状態は当日値が確定しているレースのみ（未確定なら根拠に含めない）。
    if let Some(tc) = race.track_condition
        && let Some(fs) = stat_to_triple_opt(&horse.by_track_condition, tc.as_str())
    {
        factors.push(FactorExplanation::new(
            ExplainCategory::TrackCondition,
            tc.as_str().to_string(),
            fs.rate,
            fs.starts,
        ));
    }
    // コース×枠（場×距離×馬場の枠順別）。全馬共通の course から。
    if let Some(fs) = stat_to_triple_opt(&course.by_gate_group, gate_label) {
        factors.push(FactorExplanation::new(
            ExplainCategory::CourseGate,
            gate_label.to_string(),
            fs.rate,
            fs.starts,
        ));
    }
    // 条件依存枠バイアス（馬場×頭数×枠, #343・提示専用・スコア非投入）。馬場が確定し、集計セルに実績が
    // あるときだけ提示する。lift（同条件全枠平均との差）を HorseExplanation に載せ、市場差分フラグ判定に使う。
    let mut gate_bias_lift: Option<f64> = None;
    if let Some(cg) = conditional_gate
        && let Some(tc) = race.track_condition
    {
        let track_label = gate_track_cond2_label(tc.as_str());
        let field_label = gate_field_band_label(race.field_size as u32);
        if let Some(cell) = cg.cell(track_label, field_label, gate_label)
            && cell.stat.starts > 0
        {
            factors.push(FactorExplanation::new(
                ExplainCategory::ConditionalGateBias,
                format!(
                    "{} / {} / {}",
                    gate_bias_gate_jp(gate_label),
                    track_label,
                    field_label
                ),
                RateTriple {
                    win: cell.stat.win_rate(),
                    place: cell.stat.place_rate(),
                    show: cell.stat.show_rate(),
                },
                cell.stat.starts,
            ));
            // 枠効果 lift = セル複勝率 − 同条件(馬場×頭数)の全枠平均複勝率。
            if let Some(base) = cg.condition_show_rate(track_label, field_label) {
                gate_bias_lift = Some(cell.stat.show_rate() - base);
            }
        }
    }
    // 騎手・調教師（芝ダ別）。未登録・実績なしは項を立てない。ラベルは entry の名前を使う。
    if let Some(fs) = jockey.and_then(|j| stat_to_triple_opt(&j.by_surface, surf_label)) {
        let label = entry
            .jockey
            .as_ref()
            .map(|n| n.value().to_string())
            .unwrap_or_default();
        factors.push(FactorExplanation::new(
            ExplainCategory::Jockey,
            label,
            fs.rate,
            fs.starts,
        ));
    }
    if let Some(fs) = trainer.and_then(|t| stat_to_triple_opt(&t.by_surface, surf_label)) {
        let label = entry
            .trainer
            .as_ref()
            .map(|n| n.value().to_string())
            .unwrap_or_default();
        factors.push(FactorExplanation::new(
            ExplainCategory::Trainer,
            label,
            fs.rate,
            fs.starts,
        ));
    }

    let prev_run = prev_run.map(|r| PrevRunSummary {
        finishing_position: r.result.finishing_position.map(|p| p.value()),
        popularity: r.result.popularity,
        margin: r.result.margin.clone(),
        surface: r.surface,
        distance: r.distance,
    });

    HorseExplanation {
        horse_num: entry.horse_num,
        horse_name: entry.horse_name.clone(),
        factors,
        recent_form,
        prev_run,
        gate_bias_lift,
        weight_carried: entry.weight_carried,
        field_mean_weight: race.mean_weight,
    }
}

/// 枠グループラベル（`gate_group_label` 由来の英字）を「内枠/中枠/外枠」に写す（#343 提示ラベル用）。
fn gate_bias_gate_jp(gate_label: &str) -> &'static str {
    match gate_label {
        "Inner (1-3)" => "内枠",
        "Middle (4-6)" => "中枠",
        "Outer (7-8)" => "外枠",
        _ => "枠",
    }
}

/// 斤量のレース内相対シグナル用に、出走馬の斤量[kg]の単純平均を返す（#135）。有限値が 1 つも無ければ
/// `None`（斤量項なし）。predict（出馬表 entries）と backtest（出走馬 results）で共有する。
/// 非有限値（NaN/inf）は母数から除外し、1 件の異常値が平均を NaN 化して全馬の斤量項を汚染しないようにする。
pub(crate) fn field_mean_weight(weights: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = weights
        .filter(|w| w.is_finite())
        .fold((0.0, 0u32), |(s, c), w| (s + w, c + 1));
    (n > 0).then(|| sum / n as f64)
}

/// recency 系列からラベル一致の日付系列を取り、時間減衰した重み付きレート（[`FactorStat`]）を返す。
/// ラベル不一致・有効な過去走なしは `None`（集計経路の「実績なし＝母数除外」と同じ扱い）。
fn recency_factor(
    series: &[RecencySeries],
    label: &str,
    as_of: NaiveDate,
    half_life_days: f64,
) -> Option<FactorStat> {
    series
        .iter()
        .find(|s| s.label == label)
        .and_then(|s| paddock_domain::apply_recency_weight(&s.runs, as_of, half_life_days))
}

/// label 一致の GroupStat を `FactorStat`（レート + 出走数）へ変換する。一致なし・出走 0 件は
/// `None` を返し、呼び出し側が「実績なし」を 0 レートと区別できるようにする（#73 で導入、
/// #81 で全 factor 共通化）。`starts` はベイズ縮約の信頼度重みに使う（#75）。
/// 前提: groups 内で label は一意（rdb-gateway の `group_by` が固定キーごとに 1 行生成する）。
fn stat_to_triple_opt(groups: &[GroupStat], label: &str) -> Option<FactorStat> {
    groups
        .iter()
        .find(|g| g.label == label && g.starts > 0)
        .map(|g| FactorStat {
            rate: RateTriple {
                win: g.win_rate(),
                place: g.place_rate(),
                show: g.show_rate(),
            },
            starts: g.starts,
        })
}

fn surface_label(surface: Surface) -> &'static str {
    match surface {
        Surface::Turf => "芝",
        Surface::Dirt => "ダート",
    }
}

// GateNum は 1..=8 でバリデーション済みなので _ は常に 7-8 にのみ該当する
fn gate_group_label(gate_num: u32) -> &'static str {
    match gate_num {
        1..=3 => "Inner (1-3)",
        4..=6 => "Middle (4-6)",
        _ => "Outer (7-8)",
    }
}

// ラベルは group_by_distance_band の SQL ラベル文字列と完全一致させる。
// `<= 1800` / `<= 2200` と上限を基準にすることで、SQL の BETWEEN 境界と
// 実装の意図を揃える。JRA 実レース距離は 1400m・1600m・1800m・2000m・2200m・
// 2400m 等の離散値のみで、1401〜1499m のようなレースは存在しない。
fn distance_band_label(distance: u32) -> &'static str {
    if distance <= 1400 {
        "〜1400m"
    } else if distance <= 1800 {
        "1500〜1800m"
    } else if distance <= 2200 {
        "1900〜2200m"
    } else {
        "2300m〜"
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use paddock_domain::{
        ExplainCategory, RecentRun, StandardTimes, Surface, Verdict,
        horse_result::{
            FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, ResultStatus,
        },
        race_card::HorseEntry,
    };

    use super::{RaceContext, build_explanation, recent_form_from_runs, running_style_from_runs};
    use crate::repository::{CourseStatsRow, GroupStat, HorseStatsRow};

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// shows/starts から GroupStat を作る（win/place は show と整合する適当値で埋める）。
    fn group(label: &str, starts: u32, shows: u32) -> GroupStat {
        GroupStat {
            label: label.to_string(),
            starts,
            wins: shows / 3,
            places: shows / 2,
            shows,
        }
    }

    fn empty_horse_stats() -> HorseStatsRow {
        HorseStatsRow {
            horse_name: "テスト".to_string(),
            by_surface: vec![],
            by_distance_band: vec![],
            by_gate_group: vec![],
            by_track_condition: vec![],
            by_popularity_band: vec![],
            overall: GroupStat::new("overall"),
        }
    }

    #[test]
    fn build_explanation_maps_factors_prev_and_weight() {
        let entry = HorseEntry {
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(5u32).unwrap(),
            horse_name: HorseName::try_from("テスト馬").unwrap(),
            jockey: None,
            trainer: None,
            weight_carried: Some(57.0),
        };
        let course = CourseStatsRow {
            venue: "東京".to_string(),
            distance: 1600,
            surface: "芝".to_string(),
            by_gate_group: vec![group("Inner (1-3)", 100, 23)],
        };
        let horse = HorseStatsRow {
            // 芝 複勝率 50%（10走）→ 縮約後も prior 超で Strong
            by_surface: vec![group("芝", 10, 5)],
            // 1500〜1800m 複勝率 0%（8走）→ Weak
            by_distance_band: vec![group("1500〜1800m", 8, 0)],
            ..empty_horse_stats()
        };
        let race = RaceContext {
            surface: Surface::Turf,
            distance: 1600,
            track_condition: None, // 馬場 factor なし
            field_size: 16,
            mean_weight: Some(55.0),
        };
        let prev = RecentRun {
            date: ymd(2026, 1, 6),
            surface: Surface::Turf,
            distance: 1600,
            result: HorseResult {
                finishing_position: Some(FinishingPosition::try_from(3u32).unwrap()),
                status: ResultStatus::Finished,
                gate_num: GateNum::try_from(1u32).unwrap(),
                horse_num: HorseNum::try_from(5u32).unwrap(),
                horse_name: HorseName::try_from("テスト馬").unwrap(),
                horse_id: None,
                jockey: None,
                trainer: None,
                time_seconds: None,
                margin: None,
                odds: None,
                horse_weight: None,
                weight_change: None,
                weight_carried: None,
                popularity: Some(8),
            },
            corner_positions: None,
            field_size: None,
        };

        let ex = build_explanation(
            &entry,
            &course,
            None,
            &horse,
            None,
            None,
            &race,
            Some(0.7),
            Some(&prev),
        );

        // jockey/trainer/馬場 は None なので factor は 芝・距離・枠 の 3 本。
        assert_eq!(ex.factors.len(), 3);
        assert_eq!(ex.factors[0].category, ExplainCategory::Surface);
        assert_eq!(ex.factors[0].label, "芝");
        assert_eq!(ex.factors[0].verdict, Some(Verdict::Strong));
        assert_eq!(ex.factors[1].category, ExplainCategory::Distance);
        assert_eq!(ex.factors[1].label, "1500〜1800m");
        assert_eq!(ex.factors[1].verdict, Some(Verdict::Weak));
        assert_eq!(ex.factors[2].category, ExplainCategory::CourseGate);
        assert_eq!(ex.factors[2].label, "Inner (1-3)");
        // 枠は全馬横断率なので verdict なし。
        assert_eq!(ex.factors[2].verdict, None);

        assert_eq!(ex.recent_form, Some(0.7));
        let prev_summary = ex.prev_run.expect("前走サマリがあるはず");
        assert_eq!(prev_summary.finishing_position, Some(3));
        assert_eq!(prev_summary.popularity, Some(8));
        assert_eq!(ex.weight_carried, Some(57.0));
        assert_eq!(ex.field_mean_weight, Some(55.0));
    }

    #[test]
    fn build_explanation_first_starter_has_no_factors_or_prev() {
        // 初戦馬: stats 全空・前走なし・斤量なし → factors 空・prev_run None。
        let entry = HorseEntry {
            gate_num: GateNum::try_from(4u32).unwrap(),
            horse_num: HorseNum::try_from(2u32).unwrap(),
            horse_name: HorseName::try_from("新馬").unwrap(),
            jockey: None,
            trainer: None,
            weight_carried: None,
        };
        let course = CourseStatsRow {
            venue: "東京".to_string(),
            distance: 1600,
            surface: "芝".to_string(),
            by_gate_group: vec![], // コース統計も空
        };
        let race = RaceContext {
            surface: Surface::Turf,
            distance: 1600,
            track_condition: None,
            field_size: 8,
            mean_weight: None,
        };
        let ex = build_explanation(
            &entry,
            &course,
            None,
            &empty_horse_stats(),
            None,
            None,
            &race,
            None,
            None,
        );
        assert!(ex.factors.is_empty());
        assert!(ex.prev_run.is_none());
        assert_eq!(ex.weight_carried, None);
    }

    #[test]
    fn build_explanation_adds_conditional_gate_bias_with_lift() {
        use crate::repository::{ConditionalGateStatsRow, GateBiasCell};
        use paddock_domain::TrackCondition;
        // 良・16頭(多帯)・内枠。当該セル複勝40%、中/外は20%/15% → 全枠平均=25%、lift=+15pt。
        let mk = |gate: &str, shows: u32| GateBiasCell {
            track_label: "良".to_string(),
            field_label: "多(14-18)".to_string(),
            gate_label: gate.to_string(),
            stat: group(gate, 100, shows),
        };
        let cg = ConditionalGateStatsRow {
            cells: vec![
                mk("Inner (1-3)", 40),
                mk("Middle (4-6)", 20),
                mk("Outer (7-8)", 15),
            ],
        };
        let entry = HorseEntry {
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(3u32).unwrap(),
            horse_name: HorseName::try_from("枠テスト").unwrap(),
            jockey: None,
            trainer: None,
            weight_carried: None,
        };
        let course = CourseStatsRow {
            venue: "東京".to_string(),
            distance: 1600,
            surface: "芝".to_string(),
            by_gate_group: vec![],
        };
        let race = RaceContext {
            surface: Surface::Turf,
            distance: 1600,
            track_condition: Some(TrackCondition::Firm),
            field_size: 16,
            mean_weight: None,
        };
        let ex = build_explanation(
            &entry,
            &course,
            Some(&cg),
            &empty_horse_stats(),
            None,
            None,
            &race,
            None,
            None,
        );
        let f = ex
            .factors
            .iter()
            .find(|f| f.category == ExplainCategory::ConditionalGateBias)
            .expect("条件依存枠バイアスが提示される");
        assert!((f.rate.show - 0.40).abs() < 1e-9);
        assert_eq!(f.verdict, None, "全馬横断率＝判定なし");
        assert!(
            f.label.contains("内枠") && f.label.contains("良") && f.label.contains("多(14-18)"),
            "label={}",
            f.label
        );
        // lift = 0.40 − 0.25。
        assert!(
            (ex.gate_bias_lift.unwrap() - 0.15).abs() < 1e-9,
            "lift={:?}",
            ex.gate_bias_lift
        );
    }

    #[test]
    fn build_explanation_no_gate_bias_when_track_unconfirmed() {
        use crate::repository::{ConditionalGateStatsRow, GateBiasCell};
        let cg = ConditionalGateStatsRow {
            cells: vec![GateBiasCell {
                track_label: "良".to_string(),
                field_label: "多(14-18)".to_string(),
                gate_label: "Inner (1-3)".to_string(),
                stat: group("Inner (1-3)", 100, 40),
            }],
        };
        let entry = HorseEntry {
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(3u32).unwrap(),
            horse_name: HorseName::try_from("枠テスト").unwrap(),
            jockey: None,
            trainer: None,
            weight_carried: None,
        };
        let course = CourseStatsRow {
            venue: "東京".to_string(),
            distance: 1600,
            surface: "芝".to_string(),
            by_gate_group: vec![],
        };
        // 馬場未確定(None) → 枠バイアスは提示しない（当日馬場が定まらないと条件セルを引けない）。
        let race = RaceContext {
            surface: Surface::Turf,
            distance: 1600,
            track_condition: None,
            field_size: 16,
            mean_weight: None,
        };
        let ex = build_explanation(
            &entry,
            &course,
            Some(&cg),
            &empty_horse_stats(),
            None,
            None,
            &race,
            None,
            None,
        );
        assert!(
            !ex.factors
                .iter()
                .any(|f| f.category == ExplainCategory::ConditionalGateBias)
        );
        assert_eq!(ex.gate_bias_lift, None);
    }

    fn run_valid(date: NaiveDate, weight_change: Option<i32>) -> RecentRun {
        RecentRun {
            date,
            surface: Surface::Turf,
            distance: 1600,
            result: HorseResult {
                finishing_position: Some(FinishingPosition::try_from(1u32).unwrap()),
                status: ResultStatus::Finished,
                gate_num: GateNum::try_from(1u32).unwrap(),
                horse_num: HorseNum::try_from(1u32).unwrap(),
                horse_name: HorseName::try_from("テスト").unwrap(),
                horse_id: None,
                jockey: None,
                trainer: None,
                time_seconds: None,
                margin: None,
                odds: None,
                horse_weight: None,
                weight_change,
                weight_carried: None,
                popularity: None,
            },
            corner_positions: None,
            field_size: None,
        }
    }

    /// score=None になる走を生成するヘルパー。
    /// `date=before`（cutoff 当日）で days=0 → scoring.rs が間隔シグナルを落とす。
    /// `status=DidNotFinish` + `weight_change=None` で着順・体重変化シグナルも落ちる。
    /// いずれか単独でも score=None になるが、二重に確保することでテストの堅牢性を高めている。
    fn run_no_score(before: NaiveDate) -> RecentRun {
        RecentRun {
            date: before,
            surface: Surface::Turf,
            distance: 1600,
            result: HorseResult {
                finishing_position: None,
                status: ResultStatus::DidNotFinish,
                gate_num: GateNum::try_from(1u32).unwrap(),
                horse_num: HorseNum::try_from(1u32).unwrap(),
                horse_name: HorseName::try_from("テスト").unwrap(),
                horse_id: None,
                jockey: None,
                trainer: None,
                time_seconds: None,
                margin: None,
                odds: None,
                horse_weight: None,
                weight_change: None,
                weight_carried: None,
                popularity: None,
            },
            corner_positions: None,
            field_size: None,
        }
    }

    #[test]
    fn trend_n2_both_valid_weighted_average() {
        let before = ymd(2026, 1, 20);
        // 14 日前（interval_form=1.0: scoring.rs の 14〜60 日帯）・weight_change=0（signal=1.0） → score = 1.0
        let run1 = run_valid(ymd(2026, 1, 6), Some(0));
        // 28 日前（interval_form=1.0: 同 14〜60 日帯）・weight_change=20=WEIGHT_CHANGE_CAP（上限境界値: signal=0.0） → score = 0.5
        let run2 = run_valid(ymd(2025, 12, 23), Some(20));
        let st = StandardTimes::default();
        let result = recent_form_from_runs(&[run1, run2], before, &st, 2).unwrap();
        // wsum = 1.0*1.0 + 0.5*0.5 = 1.25, wden = 1.5 → 1.25/1.5
        // 期待値は scoring.rs の WEIGHT_CHANGE_CAP=20.0・interval_form 14〜60 日=1.0 に依存（scoring 変更時は要確認）。
        let expected = 1.25_f64 / 1.5;
        assert!(
            (result - expected).abs() < 1e-9,
            "got {result}, expected {expected}"
        );
    }

    #[test]
    fn trend_n2_second_run_no_score_uses_first_only() {
        let before = ymd(2026, 1, 20);
        let run1 = run_valid(ymd(2026, 1, 6), Some(0)); // score=1.0
        let run2 = run_no_score(before); // score=None
        let st = StandardTimes::default();
        let result = recent_form_from_runs(&[run1, run2], before, &st, 2).unwrap();
        // wsum = 1.0, wden = 1.0 → 1.0
        assert!((result - 1.0).abs() < 1e-9, "got {result}");
    }

    #[test]
    fn trend_n2_all_no_score_returns_none() {
        let before = ymd(2026, 1, 20);
        let runs = vec![run_no_score(before), run_no_score(before)];
        let st = StandardTimes::default();
        assert!(recent_form_from_runs(&runs, before, &st, 2).is_none());
    }

    #[test]
    fn trend_n1_uses_only_first_run() {
        let before = ymd(2026, 1, 20);
        let run1 = run_valid(ymd(2026, 1, 6), Some(0)); // score=1.0
        let run2 = run_valid(ymd(2025, 12, 23), Some(20)); // score=0.5 (would lower if included)
        let st = StandardTimes::default();
        let result = recent_form_from_runs(&[run1, run2], before, &st, 1).unwrap();
        assert!((result - 1.0).abs() < 1e-9, "got {result}");
    }

    #[test]
    fn trend_n3_all_valid_uses_all_weights() {
        let before = ymd(2026, 1, 20);
        // score=1.0, 1.0, 0.5 の 3 走 → wsum=1.0*1+0.5*1+0.25*0.5=1.625, wden=1.75
        let run1 = run_valid(ymd(2026, 1, 6), Some(0)); // score=1.0
        let run2 = run_valid(ymd(2025, 12, 23), Some(0)); // score=1.0
        let run3 = run_valid(ymd(2025, 12, 9), Some(20)); // score=0.5
        let st = StandardTimes::default();
        let result = recent_form_from_runs(&[run1, run2, run3], before, &st, 3).unwrap();
        // wsum=1.0*1+0.5*1+0.25*0.5=1.625, wden=1.75
        // 期待値は scoring.rs の WEIGHT_CHANGE_CAP=20.0・interval_form 14〜60 日=1.0 に依存（scoring 変更時は要確認）。
        let expected = 1.625_f64 / 1.75;
        assert!(
            (result - expected).abs() < 1e-9,
            "got {result}, expected {expected}"
        );
    }

    /// 脚質テスト用に corner_positions/field_size だけ差し替えた RecentRun を作る。
    fn run_with_style(corner: Option<&str>, field_size: Option<u32>) -> RecentRun {
        RecentRun {
            corner_positions: corner.map(str::to_string),
            field_size,
            ..run_no_score(ymd(2026, 1, 1))
        }
    }

    #[test]
    fn running_style_from_runs_averages_valid_and_skips_invalid() {
        // 16 頭立て 1 番手（先行度 1.0）と 8 頭立て 8 番手（先行度 0.0）→ 平均 0.5。
        // corner/頭数を欠く走・正規化不能な走は母数から除外する。
        let runs = [
            run_with_style(Some("1-1"), Some(16)), // 1.0
            run_with_style(Some("8-7"), Some(8)),  // 0.0
            run_with_style(None, Some(16)),        // corner 欠落 → 除外
            run_with_style(Some("3-3"), None),     // 頭数欠落 → 除外
            run_with_style(Some(""), Some(16)),    // 解釈不能 → 除外
        ];
        let got = running_style_from_runs(&runs).expect("有効走 2 件で Some");
        assert!((got - 0.5).abs() < 1e-9, "got {got}");
    }

    #[test]
    fn running_style_from_runs_none_when_no_valid_run() {
        // 空・全欠落は None（母数除外・既存 scalar と統一）。
        assert!(running_style_from_runs(&[]).is_none());
        let runs = [
            run_with_style(None, Some(16)),
            run_with_style(Some("3-3"), None),
        ];
        assert!(running_style_from_runs(&runs).is_none());
    }
}
