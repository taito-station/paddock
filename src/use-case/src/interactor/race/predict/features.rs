use chrono::NaiveDate;
use paddock_domain::{
    EstimationConfig, FactorStat, HorseEntry, HorseFactors, RateTriple, RecentRun, StandardTimes,
    Surface, TrackCondition, Venue,
};

use crate::repository::{
    CourseStatsRow, GroupStat, HorseRecencyStats, HorseStatsRow, JockeyStatsRow, RecencySeries,
    TrainerStatsRow,
};

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
    /// 開催場（#350 相性 factor の venue 別成績の照合キー。`Venue::as_jp()`＝`races.venue` の値）。
    pub venue: Venue,
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

/// build_factors（score）と build_explanation（根拠）が共有する、ラベル解決済みの条件別成績（#409）。
/// 両関数が同一の (stat 行, label) → `stat_to_triple_opt` を二重評価し、手動同期でズレていた欠陥を解消する。
/// `collect_race_factors` / backtest のループで馬ごとに [`resolve_shared_factors`] で 1 回だけ構築し、
/// `&SharedFactorStats` を両関数へ渡す。10 スロットは recency 無効時の集計レート（本番・explanation はこれを使う）。
pub(crate) struct SharedFactorStats {
    // 解決済みラベル（build_explanation が FactorExplanation の label に使う。build_factors は recency 上書きに使う）。
    pub(crate) surf_label: &'static str,
    pub(crate) dist_label: &'static str,
    pub(crate) gate_label: &'static str,
    pub(crate) venue_label: &'static str,
    // 10 スロットの集計 FactorStat（母数 0・欠落・欠員は None）。
    pub(crate) course_gate: Option<FactorStat>,
    pub(crate) horse_surface: Option<FactorStat>,
    pub(crate) horse_distance: Option<FactorStat>,
    pub(crate) horse_track_condition: Option<FactorStat>,
    pub(crate) jockey_surface: Option<FactorStat>,
    pub(crate) trainer_surface: Option<FactorStat>,
    pub(crate) jockey_venue: Option<FactorStat>,
    pub(crate) jockey_distance: Option<FactorStat>,
    pub(crate) jockey_horse_combo: Option<FactorStat>,
    pub(crate) horse_venue: Option<FactorStat>,
}

/// build_factors に渡す per-horse のスカラー signal（共有の条件別 stat 以外）。#409 で引数を束ねる。
/// いずれも [0,1]（0.5=中立）または None（母数除外）。ループ側で近走・斤量から算出する。
pub(crate) struct HorseSignals {
    pub(crate) recent_form: Option<f64>,
    pub(crate) jockey_recent_form: Option<f64>,
    pub(crate) running_style: Option<f64>,
    pub(crate) weight_carried: Option<f64>,
}

/// 取得済みの stats 行から、build_factors と build_explanation が共有する条件別成績を解決する（#409）。
/// ラベル選択（`*_label`）と `stat_to_triple_opt` の組をここに集約し、両者の二重実装・手動同期を単一化する。
/// 本番 predict（全期間統計）とバックテスト（as-of 統計）の両方から共有する（ADR 0014）。
pub(crate) fn resolve_shared_factors(
    entry: &HorseEntry,
    course: &CourseStatsRow,
    horse: &HorseStatsRow,
    jockey: Option<&JockeyStatsRow>,
    trainer: Option<&TrainerStatsRow>,
    race: &RaceContext,
) -> SharedFactorStats {
    let gate_label = gate_group_label(entry.gate_num.value());
    let surf_label = surface_label(race.surface);
    let dist_label = distance_band_label(race.distance);
    // #350 相性 factor の照合キー: 競馬場は日本語場名（by_venue のラベル＝races.venue の値と一致）。
    let venue_label = race.venue.as_jp();

    // 全 factor で「実績なし」を None（母数除外）に統一する（#81/ADR 0014）。一致なし・出走 0 件は
    // stat_to_triple_opt が None を返し、0 レート（＝全敗）と区別される。jockey/trainer は
    // 騎手・調教師欠落（and_then の外側 None）と「実績なし」（内側 None）を二段で畳む。
    SharedFactorStats {
        surf_label,
        dist_label,
        gate_label,
        venue_label,
        course_gate: stat_to_triple_opt(&course.by_gate_group, gate_label),
        horse_surface: stat_to_triple_opt(&horse.by_surface, surf_label),
        horse_distance: stat_to_triple_opt(&horse.by_distance_band, dist_label),
        // 馬場状態が未確定のレース・該当馬場での出走実績が無い馬は None（#73）。
        horse_track_condition: race
            .track_condition
            .and_then(|tc| stat_to_triple_opt(&horse.by_track_condition, tc.as_str())),
        jockey_surface: jockey.and_then(|j| stat_to_triple_opt(&j.by_surface, surf_label)),
        trainer_surface: trainer.and_then(|t| stat_to_triple_opt(&t.by_surface, surf_label)),
        // #350 相性 factor（measure-first・本番は重み 0 で挙動不変）。母数 0・欠落は None。騎手系は騎手未登録も None。
        jockey_venue: jockey.and_then(|j| stat_to_triple_opt(&j.by_venue, venue_label)),
        jockey_distance: jockey.and_then(|j| stat_to_triple_opt(&j.by_distance_band, dist_label)),
        // 騎手×馬コンビ: この馬の騎手別成績（horse.by_jockey）を現騎手名で引く。騎手未登録は None。
        jockey_horse_combo: entry
            .jockey
            .as_ref()
            .and_then(|jn| stat_to_triple_opt(&horse.by_jockey, jn.value())),
        horse_venue: stat_to_triple_opt(&horse.by_venue, venue_label),
    }
}

/// 共有済み条件別成績（[`SharedFactorStats`]）と per-horse signal から `HorseFactors` を組み立てる純粋変換。
///
/// `config.recency = Some(rc)` かつ `recency` が渡されたとき、馬自身の 3 factor（芝ダ・距離帯・馬場状態）は
/// 集計レート（`shared`）の代わりに日付付き系列を時間減衰した recency 重み付きレートで評価する（#75 Phase B）。
/// `as_of_date` は減衰の基準日（predict は出馬表日、backtest はレース日）。course/jockey/trainer・相性 factor は
/// `shared` の集計レートのまま。**recency による score と根拠の乖離点はこの 1 箇所に閉じ込める**（#409）。
pub(crate) fn build_factors(
    shared: &SharedFactorStats,
    race: &RaceContext,
    signals: &HorseSignals,
    recency: Option<&HorseRecencyStats>,
    as_of_date: NaiveDate,
    config: &EstimationConfig,
) -> HorseFactors {
    // recency 有効時は horse 系 factor を日付系列の時間減衰で評価する。無効時・系列なしは共有の集計レート。
    let recency_cfg = config.recency.zip(recency);
    let horse_surface = match recency_cfg {
        Some((rc, r)) => recency_factor(
            &r.by_surface,
            shared.surf_label,
            as_of_date,
            rc.half_life_days,
        ),
        None => shared.horse_surface,
    };
    let horse_distance = match recency_cfg {
        Some((rc, r)) => recency_factor(
            &r.by_distance_band,
            shared.dist_label,
            as_of_date,
            rc.half_life_days,
        ),
        None => shared.horse_distance,
    };
    let horse_track_condition = match recency_cfg {
        Some((rc, r)) => race.track_condition.and_then(|tc| {
            recency_factor(
                &r.by_track_condition,
                tc.as_str(),
                as_of_date,
                rc.half_life_days,
            )
        }),
        None => shared.horse_track_condition,
    };

    HorseFactors {
        course_gate: shared.course_gate,
        horse_surface,
        horse_distance,
        jockey_surface: shared.jockey_surface,
        trainer_surface: shared.trainer_surface,
        // 馬場状態が未確定のレース・該当馬場での出走実績が無い馬は None（#73）。
        horse_track_condition,
        jockey_venue: shared.jockey_venue,
        jockey_distance: shared.jockey_distance,
        jockey_horse_combo: shared.jockey_horse_combo,
        horse_venue: shared.horse_venue,
        recent_form: signals.recent_form,
        weight_carried: signals.weight_carried,
        jockey_recent_form: signals.jockey_recent_form,
        running_style: signals.running_style,
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
