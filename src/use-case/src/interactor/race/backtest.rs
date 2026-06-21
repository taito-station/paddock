use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{
    BacktestReport, BettingConfig, EstimationConfig, ExoticBet, HorseEntry, HorseFactors,
    HorseOutcome, HorseResult, Podium, RaceEvaluation, ResultStatus, StandardTimes, bet_hit,
    evaluate, exotic_segments, select_bets,
};

use crate::error::Result;
use crate::interactor::Interactor;
use crate::interactor::race::predict::{
    RaceContext, build_factors, field_mean_weight, recent_form_from_runs,
};
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{OddsRepository, StatsRepository};

/// 馬番から引く発走馬の実績: `(着順, 単勝オッズ, 人気)`。いずれも欠落しうるので `Option`。
type StarterFacts = (Option<u32>, Option<f64>, Option<u32>);

impl<R: StatsRepository + OddsRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 指定期間 `[from, to]` の確定済みレースに対して確率推定を再現し、予測と実着順を突合した
    /// バックテストレポートを返す。各レース日 D の統計は `as_of = Some(D)`（`races.date < D`）で
    /// 取得するため、評価対象レース当日・以降の結果はリークしない（walk-forward）。
    ///
    /// 性能: 統計取得は **レース単位のバッチ**で行う（#196）。`as_of` はレース日ごとに変わるため
    /// 馬名でレースをまたいだバッチ化はできないが、1 レース内の出走全馬・全騎手・全調教師は同一
    /// `as_of` なので `horse_stats_batch`/`horse_recency_batch`/`jockey_stats_batch`/
    /// `trainer_stats_batch`/`recent_runs_batch` を各 1 回ずつ呼んで馬ごとの N+1 を解消する
    /// （挙動は per-item と同値）。レース取得自体の N+1 は
    /// [`Repository::find_finished_races_between`] が 2 クエリで回避済み。
    ///
    /// `blend_alpha = Some(α)` のとき、確率推定の出力を当時の市場オッズ（単勝, `as_of` 制約付き）の
    /// implied 確率と α（モデル重み）でブレンドする（#72）。`None` はモデルのみ。ブレンドは
    /// トップ選好馬・校正集計の前に適用するため、評価はブレンド後の win で行われる。
    ///
    /// `config` でベイズ縮約・リーセンシー（#75）の有効化を切り替える。`EstimationConfig::default()`
    /// は現行挙動（縮約・減衰なし）。パラメータスイープによる before/after 比較に使う。
    pub async fn backtest(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        blend_alpha: Option<f64>,
        config: EstimationConfig,
    ) -> Result<BacktestReport> {
        let races = self
            .repository
            .find_finished_races_between(from, to)
            .await?;

        let mut evaluations: Vec<RaceEvaluation> = Vec::with_capacity(races.len());
        // 買い目（curated 推奨）の券種別 校正・回収率評価用（#121）。当時 race_odds がある
        // レースのみ select_bets を回し、確定着順と突合した結果を蓄積する。
        let mut exotic_bets: Vec<ExoticBet> = Vec::new();
        // 標準タイム表は cutoff=race.date のみに依存し、同一開催日のレース間で完全に同一になる。
        // レース単位で引くと同じ集計を日数ぶん重複実行するため、日付キャッシュで 1 日 1 回に畳む（#196）。
        let mut standard_times_cache: HashMap<NaiveDate, StandardTimes> = HashMap::new();
        for race in &races {
            // 実際に発走した馬のみを評価対象（出走頭数）にする。出走取消・競走除外は本番 predict の
            // 出馬表にも載らないため、確率推定の母集合に含めると正規化分母が水増しされ確率が歪む。
            // 競走中止(DidNotFinish)は発走済みなので母集合に含め、非的中(着順なし)として扱う。
            let starters: Vec<&HorseResult> = race
                .results
                .iter()
                .filter(|r| !matches!(r.status, ResultStatus::Scratched | ResultStatus::Cancelled))
                .collect();
            // 発走馬が居なければ評価できないのでスキップ。
            if starters.is_empty() {
                continue;
            }
            let as_of = Some(race.date);

            // 回収率は実際に賭けられる「当時オッズ」で評価したい。race_odds に当該レース当日
            // 以前(date(fetched_at) <= race.date)のスナップショットがあれば使い、無ければ
            // PDF 確定成績の単勝にフォールバックする（#51）。
            let market = self.repository.find_race_odds(&race.race_id, as_of).await?;

            // コース統計は全馬共通なのでループ外で 1 回だけ取得する（predict と同じ）。
            let course = self
                .repository
                .course_stats(race.venue, race.distance, race.surface, as_of)
                .await?;

            // 斤量のレース内相対シグナル用の field 平均斤量（#135）。斤量を持つ出走馬（starters）の確定斤量で
            // 平均する（斤量欠落の馬は filter_map で母数から除外）。
            let mean_weight = field_mean_weight(starters.iter().filter_map(|r| r.weight_carried));
            let race_ctx = RaceContext {
                surface: race.surface,
                distance: race.distance,
                track_condition: race.track_condition,
                mean_weight,
            };
            // 前走タイムの相対速度シグナル用の標準タイム表（#76）。全馬共通なので horse ループ外で
            // 取得する。cutoff=race.date で walk-forward のリークを防ぐ。日付キャッシュにあれば再利用し、
            // 同一開催日のレースで集計を重複実行しない（per-item 取得と同値）。
            let standard_times = match standard_times_cache.get(&race.date) {
                Some(st) => st.clone(),
                None => {
                    let st = self.repository.standard_times(race.date).await?;
                    standard_times_cache.insert(race.date, st.clone());
                    st
                }
            };

            // 馬ごとの N+1 を解消するため、出走全馬の統計をレース単位でバッチ取得する（#196）。
            // as_of/cutoff はレース日で従来と同一。重複名はバッチ側で 1 回に dedup される。
            let horse_names: Vec<_> = starters.iter().map(|r| r.horse_name.clone()).collect();
            let jockey_names: Vec<_> = starters.iter().filter_map(|r| r.jockey.clone()).collect();
            let trainer_names: Vec<_> = starters.iter().filter_map(|r| r.trainer.clone()).collect();

            let horse_stats_map = self
                .repository
                .horse_stats_batch(&horse_names, as_of)
                .await?;
            // recency 有効時のみ日付付き系列を取得する（#75 Phase B）。無効時は空 map で済ませ、
            // per-item と同じく recency 取得自体をスキップする。
            let horse_recency_map = match config.recency {
                Some(_) => {
                    self.repository
                        .horse_recency_batch(&horse_names, as_of)
                        .await?
                }
                None => HashMap::new(),
            };
            let jockey_stats_map = self
                .repository
                .jockey_stats_batch(&jockey_names, as_of)
                .await?;
            let trainer_stats_map = self
                .repository
                .trainer_stats_batch(&trainer_names, as_of)
                .await?;
            // 前走フォーム（#31）。cutoff = race.date でレース当日以降をリークさせない。直近 1 走のみ使う。
            let recent_runs_map = self
                .repository
                .recent_runs_batch(&horse_names, race.date, 1)
                .await?;

            let mut entry_factors: Vec<(HorseEntry, HorseFactors)> = Vec::new();
            for r in &starters {
                let entry = HorseEntry {
                    gate_num: r.gate_num,
                    horse_num: r.horse_num,
                    horse_name: r.horse_name.clone(),
                    jockey: r.jockey.clone(),
                    trainer: r.trainer.clone(),
                    // 確定成績の斤量を出馬表 entry に載せ、build_factors で field 平均との相対を取る（#135）。
                    weight_carried: r.weight_carried,
                };
                // バッチ取得済みの map から引く（per-item の逐次取得と同値）。出走馬は
                // horse_stats_batch の母集合に必ず含まれるため、horse は欠落しない。
                let horse = horse_stats_map
                    .get(&r.horse_name)
                    .expect("horse_stats_batch covers all starters");
                // recency 有効時のみ日付付き系列を使う（#75 Phase B）。基準日・cutoff はレース日。
                // 無効時は map が空なので None（per-item で取得しなかったのと同値）。
                let recency = config.recency.and(horse_recency_map.get(&r.horse_name));
                let jockey = r.jockey.as_ref().map(|j| {
                    jockey_stats_map
                        .get(j)
                        .expect("jockey_stats_batch covers all starters' jockeys")
                });
                // 調教師統計（#74）。results 由来の r.trainer（当該レース確定値）から as_of で引き、
                // walk-forward のリークを防ぐ。trainer 欠落馬は項なし（ADR 0007）。
                let trainer = r.trainer.as_ref().map(|t| {
                    trainer_stats_map
                        .get(t)
                        .expect("trainer_stats_batch covers all starters' trainers")
                });
                // 前走フォーム（#31）。cutoff = race.date でレース当日以降をリークさせない。
                // バッチ取得済みの近走（直近 1 走）から純粋関数で算出する。
                let recent_runs = recent_runs_map
                    .get(&r.horse_name)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let recent_form = recent_form_from_runs(recent_runs, race.date, &standard_times);
                let factors = build_factors(
                    &entry,
                    &course,
                    horse,
                    jockey,
                    trainer,
                    &race_ctx,
                    recent_form,
                    recency,
                    race.date,
                    &config,
                );
                entry_factors.push((entry, factors));
            }

            let probs = paddock_domain::prediction::estimate_probabilities_with_config(
                &entry_factors,
                &config,
            );
            // 市場オッズ（単勝）ブレンド（#72）。α 指定時のみ適用し、以降のトップ選好馬・校正集計は
            // すべてブレンド後の win で行う。市場 win は当時 race_odds を優先し、無ければ PDF 確定
            // 成績の単勝（results.odds, 確定＝クローズ前後のオッズで結果はリークしない）で代替する。
            // 過去レースは race_odds スナップショットが無いことが多いため、この代替で評価可能になる。
            // 注意: ここで使う市場 win は回収率評価の top_pick_odds と同一ソースのため、ブレンド有効時
            // の回収率は構造的に楽観側へ寄る（probability-estimation.md 注 2）。α>=1.0 は domain 側で
            // no-op になる（predict 経路のような取得短絡は不要、market は既に取得済み）。
            let probs = match blend_alpha {
                Some(alpha) => {
                    // race_odds.win が非空ならそれを使い、完全に空のときのみ results.odds へ代替する。
                    // race_odds の win は scraper が全頭分まとめて書くため部分カバレッジは想定しないが、
                    // 仮に部分的でも results.odds へは切り替えない（blend は full coverage 前提、
                    // probability-estimation.md 参照）。
                    let market_win: HashMap<_, _> =
                        match market.as_ref().filter(|o| !o.win.is_empty()) {
                            Some(o) => o.win.iter().map(|(num, ov)| (*num, ov.value())).collect(),
                            None => starters
                                .iter()
                                .filter_map(|r| r.odds.map(|o| (r.horse_num, o)))
                                .collect(),
                        };
                    paddock_domain::prediction::blend_with_market_win(&probs, &market_win, alpha)
                }
                None => probs,
            };
            // entry_factors が非空なら estimate_probabilities は非空を返すため、理論上到達しない
            // 安全弁（空なら集計に寄与しないのでスキップ）。
            if probs.is_empty() {
                continue;
            }

            // 馬番 → (着順, 単勝オッズ, 人気) の突合表。entry_factors と同じ発走馬集合(starters)から作る。
            // 同着 1 着（稀）の場合は複数馬の won フラグが true になるが、的中率はトップ選好馬の
            // 着順のみで判定するため二重計上されない。Brier/LogLoss も破綻しない。
            let by_num: HashMap<u32, StarterFacts> = starters
                .iter()
                .map(|r| {
                    (
                        r.horse_num.value(),
                        (
                            r.finishing_position.map(|p| p.value()),
                            r.odds,
                            r.popularity,
                        ),
                    )
                })
                .collect();

            // 全馬の予測確率と実着・人気を蓄積（校正指標・reliability・セグメント用）。
            let horses: Vec<HorseOutcome> = probs
                .iter()
                .map(|p| {
                    let (finishing_position, popularity) = by_num
                        .get(&p.horse_num.value())
                        .map(|(pos, _, pop)| (*pos, *pop))
                        .unwrap_or((None, None));
                    HorseOutcome {
                        win_prob: p.win_prob,
                        place_prob: p.place_prob,
                        show_prob: p.show_prob,
                        finishing_position,
                        popularity,
                    }
                })
                .collect();

            // トップ選好馬: win_prob 最大、同値は馬番昇順。
            let top = probs
                .iter()
                .reduce(|a, b| {
                    if b.win_prob > a.win_prob
                        || (b.win_prob == a.win_prob && b.horse_num.value() < a.horse_num.value())
                    {
                        b
                    } else {
                        a
                    }
                })
                .expect("probs is non-empty");
            let (top_pick_position, pdf_top_pick_odds) = by_num
                .get(&top.horse_num.value())
                .map(|(pos, odds, _)| (*pos, *odds))
                .unwrap_or((None, None));
            // 当時オッズ（単勝）を優先し、無ければ PDF 確定成績の単勝にフォールバック。
            let market_win = market
                .as_ref()
                .and_then(|m| m.win.get(&top.horse_num))
                .map(|o| o.value());
            let top_pick_odds = market_win.or(pdf_top_pick_odds);
            // 採用したオッズ源は集計（回収率）に影響するため debug に残す（運用検証用）。
            match (market_win, top_pick_odds) {
                (Some(_), _) => {
                    tracing::debug!(race_id = %race.race_id, "backtest: 当時オッズ(単勝)を採用")
                }
                (None, Some(_)) => {
                    tracing::debug!(race_id = %race.race_id, "backtest: 当時オッズ無し、PDF 確定単勝を採用")
                }
                (None, None) => tracing::debug!(
                    race_id = %race.race_id,
                    "backtest: 単勝オッズ無し（当時・PDF とも欠落、回収率は集計対象外）"
                ),
            }

            evaluations.push(RaceEvaluation {
                horses,
                top_pick_position,
                top_pick_odds,
                surface: race.surface,
            });

            // 買い目（curated）の校正・回収率（#121）。当時 race_odds スナップショットがある
            // レースのみ対象（券種は部分的でも可。例: win のみのスナップショットなら単勝のみ評価）。
            // 本番と同じ BettingConfig::default()（curation 有）で推奨を作り、確定着順で的中判定。
            // 注意: ここに渡す probs は blend_alpha 指定時には市場 win でブレンド済みで、しかも
            // exotic の payout は同じ market のオッズで計算するため、ブレンド有効時の exotic 校正・
            // 回収率は top_pick_odds と同様に構造的に楽観側へ寄る（上の probs ブレンド注記と同根）。
            // 本番 backtest の既定は blend 無効（blend_alpha=None）でこの偏りは出ない。
            if let Some(market) = &market {
                let podium = build_podium(&starters);
                // curation は本番 predict と同じ既定値（BettingConfig::default()）固定で測る。
                // まず既定 curation の校正・回収率を定点観測するのが目的で、min_kelly /
                // max_bets_per_type を振って比較する感度分析は CLI 引数化を伴う follow-up（#122 の
                // 買い方チューニング、measurement-ordering: 既定を測ってから振る）。
                for rec in select_bets(&probs, market, &BettingConfig::default()) {
                    exotic_bets.push(ExoticBet {
                        bet_type: rec.combination.type_label(),
                        predicted_prob: rec.probability,
                        hit: bet_hit(&rec.combination, &podium),
                        odds: rec.odds,
                    });
                }
            }
        }

        let mut report = evaluate(&evaluations);
        report.by_exotic = exotic_segments(&exotic_bets);
        Ok(report)
    }
}

/// 発走馬から確定上位 3 着（着順 → 馬番）と出走頭数の [`Podium`] を作る（#121, 買い目的中判定用）。
/// `field_size` は複勝/ワイドの払戻圏（8 頭以上＝3 着・7 頭以下＝2 着）の判定に使うため発走頭数を渡す。
/// `starters` は競走除外・出走取消を除いた発走馬で、競走中止(DNF)は発走済みなので含む（JRA の
/// 出走頭数定義と一致）。同着（1〜3 着が複数馬）は同一 pos に複数該当するが `find` は先頭のみ採るため、
/// 稀な同着で組合せ券種（馬連/三連複等）や複勝/ワイドの一方馬の判定が漏れるのは許容（評価用途）。
fn build_podium(starters: &[&HorseResult]) -> Podium {
    let at = |pos: u32| {
        starters
            .iter()
            .find(|r| r.finishing_position.map(|p| p.value()) == Some(pos))
            .map(|r| r.horse_num)
    };
    Podium {
        first: at(1),
        second: at(2),
        third: at(3),
        field_size: starters.len(),
    }
}
