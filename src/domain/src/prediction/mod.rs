use std::collections::HashMap;

use chrono::NaiveDate;

use crate::horse_result::{HorseName, HorseNum, HorseResult};
use crate::race::Surface;
use crate::race_card::HorseEntry;

#[derive(Debug, Clone, Copy, Default)]
pub struct RateTriple {
    pub win: f64,
    pub place: f64,
    pub show: f64,
}

/// 1 つの factor のレート（win/place/show）と、その算出母数となった出走数（#75）。
/// `starts` はベイズ縮約（少データほど prior へ寄せる）で信頼度の重みに使う。
#[derive(Debug, Clone, Copy)]
pub struct FactorStat {
    pub rate: RateTriple,
    pub starts: u32,
}

/// ベイズ縮約（shrinkage, #75）の設定。出走数 `k` が少ない factor のレートを母集団平均
/// `PRIOR_RATE` へ `smoothed = (k·rate + m·prior)/(k + m)` で寄せ、少データ馬の過信
/// （`win_prob=0` を含む, ADR 0002）を緩和する。`pseudo_count = m` は擬似標本数。
#[derive(Debug, Clone, Copy)]
pub struct ShrinkageConfig {
    pub pseudo_count: f64,
}

/// リーセンシー重み付け（recency, #75）の設定。直近成績に時間減衰
/// `w = 0.5^(days_ago/half_life)` を掛けて集計する（Phase B で使用）。
#[derive(Debug, Clone, Copy)]
pub struct RecencyConfig {
    pub half_life_days: f64,
}

/// 確率推定の挙動切替（#75）。いずれも `None` が現行挙動（縮約・減衰なし）で、`Default` も同様。
/// backtest が CLI から組み立てて before/after を比較し、採用値を predict のデフォルトに反映する。
#[derive(Debug, Clone, Copy, Default)]
pub struct EstimationConfig {
    pub shrinkage: Option<ShrinkageConfig>,
    pub recency: Option<RecencyConfig>,
}

/// 本番 predict が採用するベイズ縮約の擬似カウント（#75）。backtest（2026-03-28〜05-31 / 144R,
/// #81 後ロジック）で m∈{off,5,10,20,50} を比較し、m=10 が単勝 Brier/LogLoss・連対で最良、
/// 的中率も改善（off 比 単勝 LogLoss 0.272→0.251、単勝的中 9.7→13.2%）だったため採用。
/// m=50 は過縮約で劣化。
pub const RECOMMENDED_SHRINKAGE_M: f64 = 10.0;

impl EstimationConfig {
    /// 本番 predict 経路のデフォルト設定（#75 で backtest 検証して採用した値）。
    /// backtest の `--shrinkage-m` 未指定（= `Default`, 縮約 off）とは別で、こちらは縮約 on。
    pub fn production() -> Self {
        Self {
            shrinkage: Some(ShrinkageConfig {
                pseudo_count: RECOMMENDED_SHRINKAGE_M,
            }),
            recency: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HorseFactors {
    /// コース（場×距離×馬場）の枠順別成績。当該コース×枠区分の出走実績が無い馬は `None`
    /// （項と重みを母数から除外、ADR 0007/0014 の欠落項扱い）。
    pub course_gate: Option<FactorStat>,
    /// 馬の芝ダ別成績。当該 surface での出走実績が無い馬（新馬等）は `None`（母数除外、#81）。
    pub horse_surface: Option<FactorStat>,
    /// 馬の距離帯別成績。当該距離帯での出走実績が無い馬（初距離等）は `None`（母数除外、#81）。
    pub horse_distance: Option<FactorStat>,
    /// 騎手の芝ダ別成績。騎手未登録、または当該 surface での騎乗実績が無い馬は `None`
    /// （母数除外、#81 で 0 埋めから統一）。
    pub jockey_surface: Option<FactorStat>,
    /// 調教師の芝ダ別成績（#74）。調教師が欠落、または当該 surface での実績が無い馬は `None`
    /// （項と重みを母数から除外、ADR 0007 の欠落項扱い）。netkeiba 出馬表からのみ取得するため、
    /// PDF 経路で取り込んだレースは常に `None`。
    pub trainer_surface: Option<FactorStat>,
    /// 馬場状態（良/稍重/重/不良）別の馬成績（#73）。対象レースの馬場状態が未確定、または
    /// 該当馬場での出走実績が無い馬は `None`（項と重みを母数から除外、ADR 0007 の欠落項扱い）。
    pub horse_track_condition: Option<FactorStat>,
    /// 前走フォーム [0,1]（0.5=中立）。前走が無い／有効な signal が無い馬は `None`。
    /// win/place/show に同値で寄与する（フォームは方向に依らず全体を底上げ／押し下げる）。
    pub recent_form: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct HorseProbability {
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    pub win_prob: f64,
    pub place_prob: f64,
    pub show_prob: f64,
}

/// 馬の過去 1 走を、その走の (surface, distance) と開催日付きで返す（#31/#76）。前走タイムを
/// 標準タイムと突き合わせて相対速度に変換するため、成績本体 `result` に加えて当該レースの
/// surface/distance を運ぶ。`find_recent_runs` の戻り要素。
#[derive(Debug, Clone)]
pub struct RecentRun {
    pub date: NaiveDate,
    pub surface: Surface,
    pub distance: u32,
    pub result: HorseResult,
}

/// コーパス由来の標準タイム表（surface×distance 別の代表タイム[秒], #76）。前走タイムを
/// 「基準タイムに対する相対速度」へ変換する分母に使う。`date < before` で集計され（as-of で
/// リーク防止）、標本数が閾値未満の薄いバケツは含めない。該当 (surface,distance) が無ければ
/// `get` が `None` を返し、タイム sub-signal は母数から落ちる（欠落フォールバック）。
#[derive(Debug, Clone, Default)]
pub struct StandardTimes {
    by_course: HashMap<(Surface, u32), f64>,
}

impl StandardTimes {
    /// (surface, distance) → 標準タイム[秒] の表から構築する。
    pub fn new(by_course: HashMap<(Surface, u32), f64>) -> Self {
        Self { by_course }
    }

    /// 指定 (surface, distance) の標準タイム[秒]。未整備なら `None`。
    pub fn get(&self, surface: Surface, distance: u32) -> Option<f64> {
        self.by_course.get(&(surface, distance)).copied()
    }
}

/// 現行挙動（縮約・減衰なし）で確率推定する。既存呼び出し・テスト互換のため signature を保つ。
pub fn estimate_probabilities(entries: &[(HorseEntry, HorseFactors)]) -> Vec<HorseProbability> {
    estimate_probabilities_with_config(entries, &EstimationConfig::default())
}

/// `config` でベイズ縮約・リーセンシーの有効化を切り替えて確率推定する（#75）。
/// `EstimationConfig::default()`（両方 `None`）は [`estimate_probabilities`] と同一挙動。
pub fn estimate_probabilities_with_config(
    entries: &[(HorseEntry, HorseFactors)],
    config: &EstimationConfig,
) -> Vec<HorseProbability> {
    if entries.is_empty() {
        return Vec::new();
    }

    let win_scores: Vec<f64> = entries
        .iter()
        .map(|(_, f)| raw_score(f, |r| r.win, config))
        .collect();
    let place_scores: Vec<f64> = entries
        .iter()
        .map(|(_, f)| raw_score(f, |r| r.place, config))
        .collect();
    let show_scores: Vec<f64> = entries
        .iter()
        .map(|(_, f)| raw_score(f, |r| r.show, config))
        .collect();

    // win は 1 着（1 ポジション）、place は 2 着以内（2 ポジション）、show は 3 着以内（3 ポジション）
    // に相当するため、レース内合計をそれぞれ 1.0 / 2.0 / 3.0 へ正規化する。各馬は確率上限 1.0。
    let win_probs = normalize_to_sum(&win_scores, 1.0);
    let mut place_probs = normalize_to_sum(&place_scores, 2.0);
    let mut show_probs = normalize_to_sum(&show_scores, 3.0);

    // 馬ごとに累積 max で単調化し win_prob ≤ place_prob ≤ show_prob を保証する。
    // win/place/show は別レートから独立に正規化するため、レート比率次第で正規化後に逆転が
    // 残りうる。これを後処理で常に是正する。
    for i in 0..place_probs.len() {
        place_probs[i] = place_probs[i].max(win_probs[i]).min(1.0);
        show_probs[i] = show_probs[i].max(place_probs[i]).min(1.0);
    }

    entries
        .iter()
        .enumerate()
        .map(|(i, (entry, _))| HorseProbability {
            horse_num: entry.horse_num,
            horse_name: entry.horse_name.clone(),
            win_prob: win_probs[i],
            place_prob: place_probs[i],
            show_prob: show_probs[i],
        })
        .collect()
}

/// 単勝確率を市場オッズ（単勝）の implied 確率とブレンドする（#72）。
///
/// `market_win_odds` は馬番→単勝確定オッズ（払戻倍率, ≥1.0）。各馬の implied 確率
/// `1/odds` をレース内で合計 1.0 に正規化（控除率＝オーバーラウンドを除去）し、モデルの
/// `win_prob` と `alpha`（モデル重み, `1-alpha` が市場重み）で線形ブレンドする。`alpha >= 1.0`
/// またはオッズが空のときはモデル確率をそのまま返す（no-op）。オッズの無い馬はブレンド時点では
/// モデル値を保つ（最後の win 合計 1.0 再正規化で全体と同じ係数でスケールはされる）。
///
/// ブレンドで win が動くため、最後に win 合計を 1.0 へ再正規化し、`place`/`show` は
/// `win ≤ place ≤ show` を保つよう累積 max で再是正する（v1 は win のみブレンド対象で
/// place/show のレートはモデル値を踏襲する）。
///
/// 前提・既知の割り切り（v1）:
/// - **(ほぼ)全頭のオッズが揃っていることを前提**とする。implied の正規化母数はオッズを持つ馬
///   のみの合計なので、一部の馬しかオッズが無い部分カバレッジでは市場重み `(1-α)` がカバー済みの
///   少数馬に偏って乗り、過大評価になりうる。実運用の単勝オッズは全頭分そろうため通常は問題ない。
/// - place/show は単調再是正のみで、**場内合計（2.0/3.0）は再正規化しない**ため、ブレンド後は
///   その合計が崩れうる。place/show の精密なブレンドは将来課題。
pub fn blend_with_market_win(
    probs: &[HorseProbability],
    market_win_odds: &HashMap<HorseNum, f64>,
    alpha: f64,
) -> Vec<HorseProbability> {
    // 非有限な α（NaN 等）は no-op 扱い（呼び出し側で検証済みだが防御的に弾く）。
    if !alpha.is_finite() {
        return probs.to_vec();
    }
    let alpha = alpha.clamp(0.0, 1.0);
    if probs.is_empty() || market_win_odds.is_empty() || alpha >= 1.0 {
        return probs.to_vec();
    }

    // 市場 implied 確率: 1/odds を合計 1.0 に正規化（オッズのある馬のみが母数）。
    // 単勝オッズ（払戻倍率）は ≥1.0。型検証を経ていない生の f64（backtest が results.odds から渡す
    // 経路）に異常値が混じっても弾けるよう doc 契約どおり `>= 1.0` でフィルタする。OddsValue 由来の
    // 経路では常に満たすが、フォールバック経路のための防御。
    let implied: HashMap<HorseNum, f64> = market_win_odds
        .iter()
        .filter(|&(_, &odds)| odds.is_finite() && odds >= 1.0)
        .map(|(&num, &odds)| (num, 1.0 / odds))
        .collect();
    let overround: f64 = implied.values().sum();
    if overround <= 0.0 {
        return probs.to_vec();
    }

    // モデル win と市場 implied をブレンド（オッズの無い馬はモデル値のまま）。
    let blended: Vec<f64> = probs
        .iter()
        .map(|p| match implied.get(&p.horse_num) {
            Some(&imp) => alpha * p.win_prob + (1.0 - alpha) * (imp / overround),
            None => p.win_prob,
        })
        .collect();

    // 部分カバレッジや凸結合のドリフトを吸収して win 合計を 1.0 へ戻す。
    // `min(1.0)` は w ≤ total（全要素非負）より数学的には恒等だが、浮動小数点の保険として残す。
    let total: f64 = blended.iter().sum();
    let win_probs: Vec<f64> = if total > 0.0 {
        blended.iter().map(|w| (w / total).min(1.0)).collect()
    } else {
        blended
    };

    probs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let win = win_probs[i];
            let place = p.place_prob.max(win).min(1.0);
            let show = p.show_prob.max(place).min(1.0);
            HorseProbability {
                horse_num: p.horse_num,
                horse_name: p.horse_name.clone(),
                win_prob: win,
                place_prob: place,
                show_prob: show,
            }
        })
        .collect()
}

const COURSE_GATE_WEIGHT: f64 = 2.0;
const SURFACE_WEIGHT: f64 = 1.0;
const DISTANCE_WEIGHT: f64 = 1.0;
const JOCKEY_WEIGHT: f64 = 1.0;
/// 調教師（trainer）項の重み。#87 で母数（results.trainer）を充足し backtest（0.0/0.5/1.0/2.0 を
/// 比較, 2026-03-28〜05-31 / 144 レース, #81 後ロジック）で再検証した。項を有効化すると校正が改善
/// （0.0→0.5 で LogLoss 単勝 0.60→0.40、Brier 系は小幅）。0.5/1.0/2.0 は拮抗で、1.0 が LogLoss 単勝・
/// Brier 複勝で最良（Brier 単勝のみ 2.0 が僅差だが小標本ゆえ過適合回避で 1.0）。jockey と同値（ADR 0012）。
const TRAINER_WEIGHT: f64 = 1.0;
/// 馬場状態（track_condition）項の重み。#73 バックテスト（0.25/0.5/1.0/1.5/2.0 を比較）で
/// 1.0 が的中率・回収率のピークだったため採用（ADR 0011）。
const TRACK_CONDITION_WEIGHT: f64 = 1.0;
/// 前走フォーム項の重み。#30 バックテストで検証して決定（ADR 0009）。
const FORM_WEIGHT: f64 = 0.25;

/// ベイズ縮約（#75）の母集団 prior レート。出走頭数の代表値（≒14 頭）から導く解析的な基準率
/// （win=1/14, place=2/14, show=3/14）で、「平均的な 1 頭が 1 着/2 着内/3 着内に入る確率」に相当する。
/// 実績の薄い factor のレートをこの prior へ寄せる。クエリ不要でリークが無い最小実装。将来は
/// results 全体の実測ベースレートへ差し替え可能（backtest で要否を再検証）。
const PRIOR_RATE: RateTriple = RateTriple {
    win: 1.0 / 14.0,
    place: 2.0 / 14.0,
    show: 3.0 / 14.0,
};

/// ベイズ縮約: 出走数 `starts`(=k) の少ない factor のレートを prior へ寄せる（#75）。
/// `smoothed = (k·rate + m·prior) / (k + m)`。k≫m で ≈rate、k=0 で =prior、単調に補間する。
fn shrink_rate(rate: f64, starts: u32, prior: f64, pseudo_count: f64) -> f64 {
    let k = starts as f64;
    (k * rate + pseudo_count * prior) / (k + pseudo_count)
}

/// 日付付きの 1 日分（または同一日複数走）の成績カウント（リーセンシー重み付け用, #75 Phase B）。
#[derive(Debug, Clone, Copy)]
pub struct DatedCounts {
    pub date: NaiveDate,
    pub starts: u32,
    pub wins: u32,
    pub places: u32,
    pub shows: u32,
}

/// 日付付き成績系列に時間減衰 `w = 0.5^((as_of − date)/half_life)` を掛け、時間重み付きレート
/// （`Σ w·wins / Σ w·starts` 等）と総出走数を `FactorStat` で返す（#75 Phase B）。直近走ほど
/// 重みが大きく、半減期 `half_life_days` 日で寄与が半分になる。`as_of` 以降の日付はリーク防止の
/// ため無視する（呼び出し側が as_of で絞るが二重防御）。有効な重み付き出走が無ければ `None`。
///
/// `FactorStat.starts` は時間重みを掛けない素の総出走数を返す。recency と shrinkage を併用すると
/// 縮約はこの素の starts を信頼度 k に使う（＝減衰で薄れた古い実績も母数に満額カウント）。この
/// 非対称は割り切りで、併用経路は backtest（CLI 両指定）でのみ到達し本番 predict では走らない
/// （`production()` は recency 無効）。recency を将来採用する際は減衰後の実効標本数での縮約を
/// 再検討する（ADR 0016）。
pub fn apply_recency_weight(
    runs: &[DatedCounts],
    as_of: NaiveDate,
    half_life_days: f64,
) -> Option<FactorStat> {
    // 呼び出し側（CLI `--recency-half-life`）が有限の正数を保証する。万一 0・負・非有限が来ても
    // `0.5^(±inf)` 等で全重み 0 → None に倒れ NaN は出さないが、契約違反は debug ビルドで検出する。
    debug_assert!(
        half_life_days.is_finite() && half_life_days > 0.0,
        "half_life_days must be finite and positive, got {half_life_days}"
    );
    let mut w_starts = 0.0;
    let mut w_wins = 0.0;
    let mut w_places = 0.0;
    let mut w_shows = 0.0;
    let mut total_starts: u32 = 0;
    for r in runs {
        let days_ago = (as_of - r.date).num_days();
        // as_of 当日・以降はリークになるため寄与させない（< as_of のみ）。
        if days_ago <= 0 {
            continue;
        }
        let w = 0.5_f64.powf(days_ago as f64 / half_life_days);
        w_starts += w * r.starts as f64;
        w_wins += w * r.wins as f64;
        w_places += w * r.places as f64;
        w_shows += w * r.shows as f64;
        // 実データでは 1 頭の生涯出走数は高々数十だが、契約外の入力でも安全側に倒す。
        total_starts = total_starts.saturating_add(r.starts);
    }
    if w_starts <= 0.0 {
        return None;
    }
    Some(FactorStat {
        rate: RateTriple {
            win: w_wins / w_starts,
            place: w_places / w_starts,
            show: w_shows / w_starts,
        },
        starts: total_starts,
    })
}

/// 1 つの factor の寄与レートを返す。`config.shrinkage` が `Some` のときはベイズ縮約を適用し、
/// `None` のときは生レート（現行挙動）。`rate` セレクタは win/place/show のいずれかを取り出す。
fn factor_value(fs: &FactorStat, rate: fn(&RateTriple) -> f64, config: &EstimationConfig) -> f64 {
    let raw = rate(&fs.rate);
    match config.shrinkage {
        Some(s) => shrink_rate(raw, fs.starts, rate(&PRIOR_RATE), s.pseudo_count),
        None => raw,
    }
}

/// 存在する factor の**重み付き平均**を返す。実績の無い項（出走実績なし・騎手未登録・前走なし等）は
/// その項と重みを母数から除外して評価するため、欠落で不当に減点されない（ADR 0007/0014）。
/// 「実績なし」を 0 レート（＝全敗）と同一視しない方針を全 factor に統一する（#81）。全馬が同条件の
/// ときは定数除算となり、レース内正規化後の相対順位は変わらない。
///
/// 全 factor が欠落（`weight == 0.0`）の馬はゼロ除算（NaN）を避けて `0.0` を返す。score 0 の馬は
/// `normalize_to_sum` の全 0 フォールバックで均等確率に畳まれる。
///
/// `recent_form` はスカラー（[0,1]、0.5=中立）で win/place/show に同値で寄与する。
fn raw_score(
    factors: &HorseFactors,
    rate: fn(&RateTriple) -> f64,
    config: &EstimationConfig,
) -> f64 {
    let mut weighted = 0.0;
    let mut weight = 0.0;
    if let Some(course_gate) = factors.course_gate {
        weighted += COURSE_GATE_WEIGHT * factor_value(&course_gate, rate, config);
        weight += COURSE_GATE_WEIGHT;
    }
    if let Some(surface) = factors.horse_surface {
        weighted += SURFACE_WEIGHT * factor_value(&surface, rate, config);
        weight += SURFACE_WEIGHT;
    }
    if let Some(distance) = factors.horse_distance {
        weighted += DISTANCE_WEIGHT * factor_value(&distance, rate, config);
        weight += DISTANCE_WEIGHT;
    }
    if let Some(jockey) = factors.jockey_surface {
        // 騎手も全 factor 共通の縮約 m を使う。騎手専用の強い縮約（小サンプル過信の抑制）は
        // #105 で backtest 評価したが集約指標に改善が無く（むしろ微悪化）採用見送り（ADR 0017）。
        weighted += JOCKEY_WEIGHT * factor_value(&jockey, rate, config);
        weight += JOCKEY_WEIGHT;
    }
    if let Some(trainer) = factors.trainer_surface {
        weighted += TRAINER_WEIGHT * factor_value(&trainer, rate, config);
        weight += TRAINER_WEIGHT;
    }
    if let Some(tc) = factors.horse_track_condition {
        weighted += TRACK_CONDITION_WEIGHT * factor_value(&tc, rate, config);
        weight += TRACK_CONDITION_WEIGHT;
    }
    if let Some(form) = factors.recent_form {
        weighted += FORM_WEIGHT * form;
        weight += FORM_WEIGHT;
    }
    if weight == 0.0 {
        return 0.0;
    }
    weighted / weight
}

/// スコアをレース内合計が `target` になるよう正規化し、各値を確率として `[0, 1]` にクランプする。
/// 全スコアが 0（出走馬全員のスタッツ未蓄積）の場合は均等フォールバック `target / n`（上限 1.0）。
fn normalize_to_sum(scores: &[f64], target: f64) -> Vec<f64> {
    let n = scores.len();
    let total: f64 = scores.iter().sum();
    if total <= 0.0 {
        let each = (target / n as f64).min(1.0);
        return vec![each; n];
    }
    scores
        .iter()
        .map(|s| (s / total * target).min(1.0))
        .collect()
}

/// 馬体重変化がこの kg を超えると不安定として最低評価（0）にする。
const WEIGHT_CHANGE_CAP: f64 = 20.0;
/// 前走の人気順位と着順の差 1 つあたりのスコア寄与。
const POP_GAP_K: f64 = 0.08;
/// 前走着差（馬身）がこの値以上で競争力差を最大とみなすクランプ点（大差勝ち・大敗の上限, #76）。
/// 暫定値。backtest（main との before/after 比較）で寄与を確認して調整する。
const MARGIN_CAP_LENGTHS: f64 = 5.0;
/// 前走タイムの相対速度 signal（#76）の飽和上限。標準タイムからの相対偏差
/// `(standard - prev) / standard` がこの割合（例 0.05 = ±5%）で signal が 0/1 に飽和する。
/// レース内のタイム差は数 % に収まるため小さめに置く。暫定値で backtest（main との before/after）
/// で寄与を確認して調整する。
const TIME_DEV_CAP: f64 = 0.05;

/// 直近 1 走（`prev`、その開催日 `prev_date`）と対象レース日 `race_date` から「前走フォーム」
/// スコア `[0,1]`（0.5=中立）を算出する。利用できる sub-signal（馬体重変化・前走人気乖離・前走間隔・
/// 前走着差・前走タイム）の平均を返す。有効な signal が 1 つも無い場合は `None`（前走情報が乏しい→
/// スコアに寄与させない）。
///
/// `standard_time` は前走の (surface, distance) に対するコーパス標準タイム[秒]（#76）。前走タイムを
/// 相対速度シグナルに変換する分母で、呼び出し側が `StandardTimes::get` で解決して渡す。前走タイムが
/// 無い／標準タイムが未整備（`None`）のときはタイム sub-signal を落とす（欠落フォールバック）。
pub fn recent_form_score(
    prev: &HorseResult,
    prev_date: NaiveDate,
    race_date: NaiveDate,
    standard_time: Option<f64>,
) -> Option<f64> {
    let mut signals: Vec<f64> = Vec::new();

    // 馬体重変化: |Δkg| が小さいほど安定＝良。CAP 超で 0。
    if let Some(dw) = prev.weight_change {
        signals.push(1.0 - (dw.unsigned_abs() as f64 / WEIGHT_CHANGE_CAP).min(1.0));
    }

    // 前走人気乖離: 人気順位より好走（着順が人気順位より小さい）で加点、凡走で減点。
    // 着順なし（中止・失格・取消で finishing_position = None）の前走は乖離を測れないため、
    // この signal を落として残りの signal（体重・間隔）で評価する。
    if let (Some(pop), Some(pos)) = (prev.popularity, prev.finishing_position.map(|p| p.value())) {
        let gap = pop as f64 - pos as f64; // >0: 人気以上の好走
        signals.push((0.5 + gap * POP_GAP_K).clamp(0.0, 1.0));
    }

    // 前走間隔: 中2週(14)〜2ヶ月(60)を最適(1.0)、連闘(<14)/長休(>120)を逓減。
    // 本番経路では find_recent_runs が `races.date < before` で前走のみ返すため days は常に正。
    // `days > 0` は異常データ（同日/未来の前走）に対する防御で、その場合は間隔 signal を落とす。
    let days = (race_date - prev_date).num_days();
    if days > 0 {
        signals.push(interval_form(days));
    }

    // 前走着差: 圧勝＝強い／大敗＝弱い（#76）。着順なし（中止・失格・取消）や着差文字列が
    // 解釈不能・空の前走はこの signal を落とし、残りの signal で評価する（欠落フォールバック）。
    if let (Some(pos), Some(len)) = (
        prev.finishing_position.map(|p| p.value()),
        prev.margin.as_deref().and_then(parse_margin_lengths),
    ) {
        signals.push(margin_form(pos, len));
    }

    // 前走タイム: 同一 (surface,distance) のコーパス標準タイムに対する相対速度（#76）。標準より
    // 速い＝強いで加点、遅い＝弱いで減点。タイム無し（中止・失格や未記録）や標準タイム未整備は
    // sub-signal を落とし、残りの signal で評価する（欠落フォールバック）。`t > 0` は 0 秒の異常値
    // （TimeSeconds は 0.0 を許容）を母数から落とす防御で、標準タイム集計側の `time_seconds > 0` と揃える。
    if let (Some(t), Some(std)) = (prev.time_seconds.map(|x| x.value()), standard_time) {
        if t > 0.0 {
            signals.push(time_form(t, std));
        }
    }

    if signals.is_empty() {
        None
    } else {
        Some(signals.iter().sum::<f64>() / signals.len() as f64)
    }
}

/// 前走間隔（日数）→ `[0,1]` の台形マップ。区分は離散で、境界に小さな段差がある（heuristic）。
fn interval_form(days: i64) -> f64 {
    match days {
        d if d <= 7 => 0.3,                                  // 連闘・中1週未満
        d if d < 14 => 0.3 + 0.7 * (d - 7) as f64 / 7.0,     // 8〜13 日: 0.3→0.9 にランプ
        d if d <= 60 => 1.0,                                 // 中2週〜2ヶ月: 最適帯
        d if d <= 120 => 1.0 - 0.5 * (d - 60) as f64 / 60.0, // 60→120 日で 1.0→0.5
        _ => 0.5,                                            // 長期休み明け（不確実）
    }
}

/// 前走着差（馬身）と前走着順から「前走の競争力」シグナル `[0,1]`（0.5=中立）を作る（#76）。
/// 勝ち（1 着）は着差が大きいほど圧勝＝強い（0.5→1.0）、負けは前を行く馬への着差が大きいほど
/// 大敗＝弱い（0.5→0.0）。JRA/netkeiba の着差はその馬と「直前に入線した馬」との局所差であり
/// 1 着馬からの累積差ではない。負け馬の評価はこの局所差を流用する割り切り（heuristic）で、
/// 寄与の要否は backtest（main との before/after 比較）で判定する。
///
/// 非対称性の注意: JRA PDF 経路では勝ち馬の着差列はブランクで margin=None になる（パーサが
/// タイム直後の通過順位を着差として弾く）。そのため PDF 由来データでは `position == 1`（加点）
/// ブランチは実質不活性で、本シグナルは主に「大敗の負け馬を減点」する向きに効く。勝ち馬の加点は
/// 着差を持つ netkeiba 履歴の取り込み後に機能する。
fn margin_form(position: u32, margin_lengths: f64) -> f64 {
    let mag = (margin_lengths / MARGIN_CAP_LENGTHS).clamp(0.0, 1.0);
    if position == 1 {
        0.5 + 0.5 * mag
    } else {
        0.5 - 0.5 * mag
    }
}

/// 前走タイム `prev_time`[秒] とコーパス標準タイム `standard_time`[秒] から「前走の相対速度」
/// シグナル `[0,1]`（0.5=中立）を作る（#76）。標準より速い（タイムが小さい）ほど高く、遅いほど低い。
/// 相対偏差 `dev = (standard - prev) / standard` を `TIME_DEV_CAP` で飽和させて線形に写像する。
/// 馬場差は標準タイム集計時に (surface,distance) でプールして吸収する割り切り（v1）。
/// 標準タイムが非正のときは比が定義できないため中立 0.5 を返す（防御）。
fn time_form(prev_time: f64, standard_time: f64) -> f64 {
    if standard_time <= 0.0 {
        return 0.5;
    }
    let dev = (standard_time - prev_time) / standard_time;
    (0.5 + 0.5 * dev / TIME_DEV_CAP).clamp(0.0, 1.0)
}

/// 前走着差文字列を馬身（length）に変換する（#76）。複数出典の表記を吸収する:
/// キーワード（`ハナ`/`アタマ`/`クビ`/`大差`/`同着`）、分数（`3/4`・整数+分数 `1.1/4`）、
/// 小数・整数（`0.6`/`2`）。解釈できない・空文字・負値は `None`（signal を母数から除外）。
fn parse_margin_lengths(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    // キーワード表記。PDF パーサはハナ/アタマ/クビのみ、netkeiba は大差・同着も返す。
    // 馬身換算は JRA の慣行値（ハナ<アタマ<クビ）に倣う近似。
    if t.contains("同着") {
        return Some(0.0);
    }
    if t.contains("大差") {
        // クランプ点と同じ定数を返し、「大差」を必ず競争力差の最大（margin_form で mag=1.0）に揃える。
        // 片方だけ調整すると意図がずれるため二役であることを明示。
        return Some(MARGIN_CAP_LENGTHS);
    }
    if t.contains("ハナ") {
        return Some(0.05);
    }
    if t.contains("アタマ") {
        return Some(0.10);
    }
    if t.contains("クビ") {
        return Some(0.25);
    }
    // 分数表記。`/` を含むとき、`.` があれば整数部+分数部（`1.1/4` = 1 + 1/4）、無ければ分数のみ。
    if t.contains('/') {
        if let Some(dot) = t.find('.') {
            let whole: f64 = t[..dot].trim().parse().ok()?;
            let frac = parse_fraction(&t[dot + 1..])?;
            return Some(whole + frac);
        }
        return parse_fraction(t);
    }
    // 小数・整数（`0.6` / `2` / `1.0`）。負値・非有限は弾く。
    t.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)
}

/// `A/B` 形式の分数文字列を解釈する。パース不能・分母 0・負値・非有限は `None`
/// （小数経路と同じく着差は非負のみ受ける）。
fn parse_fraction(s: &str) -> Option<f64> {
    let (num, den) = s.split_once('/')?;
    let num: f64 = num.trim().parse().ok()?;
    let den: f64 = den.trim().parse().ok()?;
    if den == 0.0 {
        return None;
    }
    let v = num / den;
    (v.is_finite() && v >= 0.0).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horse_result::{
        FinishingPosition, GateNum, HorseName, HorseNum, ResultStatus, TimeSeconds,
    };
    use crate::race_card::HorseEntry;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
    }

    fn prev_result(
        weight_change: Option<i32>,
        popularity: Option<u32>,
        finish: Option<u32>,
    ) -> HorseResult {
        HorseResult {
            finishing_position: finish.map(|p| FinishingPosition::try_from(p).unwrap()),
            status: ResultStatus::Finished,
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(1u32).unwrap(),
            horse_name: HorseName::try_from("ウマ").unwrap(),
            horse_id: None,
            jockey: None,
            trainer: None,
            time_seconds: None,
            margin: None,
            odds: None,
            horse_weight: None,
            weight_change,
            weight_carried: None,
            popularity,
        }
    }

    fn make_entry(horse_num: u32, horse_name: &str) -> HorseEntry {
        HorseEntry {
            gate_num: crate::horse_result::GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(horse_num).unwrap(),
            horse_name: HorseName::try_from(horse_name).unwrap(),
            jockey: None,
            trainer: None,
        }
    }

    /// テスト用: レートを `FactorStat`（出走数 10）に包む。ベイズ縮約 off（`EstimationConfig::default`）
    /// の挙動不変テストでは `starts` の値は結果に影響しない。縮約挙動のテストでは starts を明示する。
    fn fs(rate: RateTriple) -> FactorStat {
        FactorStat { rate, starts: 10 }
    }

    fn zero_factors() -> HorseFactors {
        HorseFactors {
            course_gate: Some(fs(RateTriple::default())),
            horse_surface: Some(fs(RateTriple::default())),
            horse_distance: Some(fs(RateTriple::default())),
            jockey_surface: None,
            horse_track_condition: None,
            trainer_surface: None,
            recent_form: None,
        }
    }

    #[test]
    fn empty_entries() {
        let result = estimate_probabilities(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn uniform_fallback_when_all_scores_zero() {
        let entries = vec![
            (make_entry(1, "ウマA"), zero_factors()),
            (make_entry(2, "ウマB"), zero_factors()),
            (make_entry(3, "ウマC"), zero_factors()),
        ];
        let probs = estimate_probabilities(&entries);
        assert_eq!(probs.len(), 3);
        // win=1/3, place=2/3, show=3/3=1.0（3 頭立てなら全馬が複勝圏）。すべて単調。
        for p in &probs {
            assert!((p.win_prob - 1.0 / 3.0).abs() < 1e-10);
            assert!((p.place_prob - 2.0 / 3.0).abs() < 1e-10);
            assert!((p.show_prob - 1.0).abs() < 1e-10);
            assert!(p.win_prob <= p.place_prob && p.place_prob <= p.show_prob);
        }
        let win_total: f64 = probs.iter().map(|p| p.win_prob).sum();
        assert!((win_total - 1.0).abs() < 1e-10);
    }

    #[test]
    fn all_factors_none_scores_zero_and_falls_back_uniform() {
        // 全 factor 欠落（どの統計も実績なし）の馬は weight==0 → raw_score=0.0（NaN でない）。
        let none_factors = HorseFactors {
            course_gate: None,
            horse_surface: None,
            horse_distance: None,
            jockey_surface: None,
            horse_track_condition: None,
            trainer_surface: None,
            recent_form: None,
        };
        // assert_eq! は NaN（0/0 のゼロ除算）でも 0.0 と不一致で失敗するため NaN 回避も兼ねる。
        let s = raw_score(&none_factors, |r| r.win, &EstimationConfig::default());
        assert_eq!(s, 0.0, "all-None must score finite 0.0, got {s}");

        // estimate_probabilities は全スコア 0 → 均等フォールバック（2 頭なら win=0.5）。
        let entries = vec![
            (make_entry(1, "ウマA"), none_factors.clone()),
            (make_entry(2, "ウマB"), none_factors),
        ];
        let probs = estimate_probabilities(&entries);
        for p in &probs {
            assert!((p.win_prob - 0.5).abs() < 1e-10);
            assert!(p.win_prob <= p.place_prob && p.place_prob <= p.show_prob);
        }
    }

    /// #81 の核心: 「実績なし」を `None`（母数除外）にすると、0 埋め（`Some(0-rate)`＝全敗扱い）
    /// より不当に減点されないこと。他 factor が等しく正のレートなら、除外馬はその平均を維持する。
    #[test]
    fn missing_record_excluded_is_not_penalized_like_zero_fill() {
        let base = RateTriple {
            win: 0.2,
            place: 0.4,
            show: 0.6,
        };
        // horse_surface の実績なし → None（母数除外）。残り course_gate/distance の平均 0.2 を維持。
        let excluded = HorseFactors {
            course_gate: Some(fs(base)),
            horse_surface: None,
            horse_distance: Some(fs(base)),
            jockey_surface: None,
            horse_track_condition: None,
            trainer_surface: None,
            recent_form: None,
        };
        // 旧挙動相当: horse_surface=Some(0-rate) は母数に残り平均を押し下げる（＝減点）。
        let zero_filled = HorseFactors {
            horse_surface: Some(fs(RateTriple::default())),
            ..excluded.clone()
        };
        let s_excluded = raw_score(&excluded, |r| r.win, &EstimationConfig::default());
        let s_zero = raw_score(&zero_filled, |r| r.win, &EstimationConfig::default());
        assert!((s_excluded - 0.2).abs() < 1e-10, "excluded={s_excluded}");
        assert!(
            s_excluded > s_zero,
            "実績なし除外は 0 埋めより高評価であるべき: excluded={s_excluded}, zero={s_zero}"
        );
    }

    #[test]
    fn win_sums_to_one_and_values_monotone_small_field() {
        let entries = vec![
            (
                make_entry(1, "ウマA"),
                HorseFactors {
                    course_gate: Some(fs(RateTriple {
                        win: 0.2,
                        place: 0.4,
                        show: 0.6,
                    })),
                    horse_surface: Some(fs(RateTriple {
                        win: 0.1,
                        place: 0.2,
                        show: 0.3,
                    })),
                    horse_distance: Some(fs(RateTriple {
                        win: 0.15,
                        place: 0.3,
                        show: 0.45,
                    })),
                    jockey_surface: None,
                    horse_track_condition: None,
                    trainer_surface: None,
                    recent_form: None,
                },
            ),
            (
                make_entry(2, "ウマB"),
                HorseFactors {
                    course_gate: Some(fs(RateTriple {
                        win: 0.1,
                        place: 0.2,
                        show: 0.3,
                    })),
                    horse_surface: Some(fs(RateTriple {
                        win: 0.05,
                        place: 0.1,
                        show: 0.15,
                    })),
                    horse_distance: Some(fs(RateTriple {
                        win: 0.08,
                        place: 0.16,
                        show: 0.24,
                    })),
                    jockey_surface: None,
                    horse_track_condition: None,
                    trainer_surface: None,
                    recent_form: None,
                },
            ),
        ];
        let probs = estimate_probabilities(&entries);
        assert_eq!(probs.len(), 2);
        // win は 1 着＝1 ポジションなので合計 ≒ 1.0。place/show は小頭数だと上限 1.0 クランプで
        // 合計が 2/3 を下回りうるため、ここでは各値が [0,1] かつ単調であることを確認する。
        let win_total: f64 = probs.iter().map(|p| p.win_prob).sum();
        assert!((win_total - 1.0).abs() < 1e-10);
        for p in &probs {
            assert!((0.0..=1.0).contains(&p.win_prob));
            assert!((0.0..=1.0).contains(&p.place_prob));
            assert!((0.0..=1.0).contains(&p.show_prob));
            assert!(p.win_prob <= p.place_prob && p.place_prob <= p.show_prob);
        }
    }

    /// 上限クランプが起きない十分大きい均等フィールドでは place 合計 ≒ 2.0、show 合計 ≒ 3.0。
    #[test]
    fn place_show_sum_to_two_and_three_in_even_field() {
        let triple = RateTriple {
            win: 0.1,
            place: 0.2,
            show: 0.3,
        };
        let factors = HorseFactors {
            course_gate: Some(fs(triple)),
            horse_surface: Some(fs(triple)),
            horse_distance: Some(fs(triple)),
            jockey_surface: None,
            horse_track_condition: None,
            trainer_surface: None,
            recent_form: None,
        };
        // 6 頭立て・全馬同一スコア → win=1/6, place=2/6, show=3/6（いずれも 1.0 未満で無クランプ）。
        let entries: Vec<_> = (1..=6)
            .map(|i| (make_entry(i, &format!("ウマ{i}")), factors.clone()))
            .collect();
        let probs = estimate_probabilities(&entries);
        let win_total: f64 = probs.iter().map(|p| p.win_prob).sum();
        let place_total: f64 = probs.iter().map(|p| p.place_prob).sum();
        let show_total: f64 = probs.iter().map(|p| p.show_prob).sum();
        assert!((win_total - 1.0).abs() < 1e-9, "win_total={win_total}");
        assert!(
            (place_total - 2.0).abs() < 1e-9,
            "place_total={place_total}"
        );
        assert!((show_total - 3.0).abs() < 1e-9, "show_total={show_total}");
    }

    /// win レートが高く place/show レートが相対的に低い馬でも、後処理の累積 max で
    /// win ≤ place ≤ show が必ず成立する。
    #[test]
    fn monotonicity_guaranteed_even_with_inverted_rates() {
        // ウマA: win 偏重（place/show が win より低い不自然なレート）。ウマB: 逆。
        let a = HorseFactors {
            course_gate: Some(fs(RateTriple {
                win: 0.9,
                place: 0.1,
                show: 0.1,
            })),
            horse_surface: Some(fs(RateTriple::default())),
            horse_distance: Some(fs(RateTriple::default())),
            jockey_surface: None,
            horse_track_condition: None,
            trainer_surface: None,
            recent_form: None,
        };
        let b = HorseFactors {
            course_gate: Some(fs(RateTriple {
                win: 0.1,
                place: 0.9,
                show: 0.9,
            })),
            horse_surface: Some(fs(RateTriple::default())),
            horse_distance: Some(fs(RateTriple::default())),
            jockey_surface: None,
            horse_track_condition: None,
            trainer_surface: None,
            recent_form: None,
        };
        let entries = vec![(make_entry(1, "ウマA"), a), (make_entry(2, "ウマB"), b)];
        let probs = estimate_probabilities(&entries);
        for p in &probs {
            assert!(
                p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
                "non-monotonic: {p:?}"
            );
        }
    }

    /// 列ごとに独立してフォールバック判定するため、一部の列だけ合計 0 になりうる
    /// （例: 全馬 place=show=0 だが win>0）。合計 0 の列は均等フォールバック（place→min(2/n,1)、
    /// show→min(3/n,1)）になり、累積 max により win ≤ place ≤ show は保たれる。
    #[test]
    fn monotonic_when_only_some_columns_are_all_zero() {
        // win レートのみ非ゼロ、place/show レートは全馬 0。
        let win_only = |w: f64| HorseFactors {
            course_gate: Some(fs(RateTriple {
                win: w,
                place: 0.0,
                show: 0.0,
            })),
            horse_surface: Some(fs(RateTriple::default())),
            horse_distance: Some(fs(RateTriple::default())),
            jockey_surface: None,
            horse_track_condition: None,
            trainer_surface: None,
            recent_form: None,
        };
        let entries = vec![
            (make_entry(1, "ウマA"), win_only(0.3)),
            (make_entry(2, "ウマB"), win_only(0.1)),
        ];
        let probs = estimate_probabilities(&entries);
        for p in &probs {
            assert!(
                p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
                "non-monotonic: {p:?}"
            );
            assert!((0.0..=1.0).contains(&p.place_prob));
            assert!((0.0..=1.0).contains(&p.show_prob));
        }
        // 2 頭立てでは place/show 列が合計 0 → 均等フォールバックで min(2/2,1)=min(3/2,1)=1.0。
        for p in &probs {
            assert!((p.place_prob - 1.0).abs() < 1e-10);
            assert!((p.show_prob - 1.0).abs() < 1e-10);
        }
        // win 列は非ゼロなので通常正規化（合計 1.0）。
        let win_total: f64 = probs.iter().map(|p| p.win_prob).sum();
        assert!((win_total - 1.0).abs() < 1e-10);
    }

    /// 騎手なし馬が欠落項で不当に減点されないこと（重み付き平均）。レートが全 factor で等しいなら
    /// 騎手の有無でスコアは変わらず、騎手項は「平均からの差」としてのみ効く。
    #[test]
    fn jockey_none_not_penalized() {
        let base = RateTriple {
            win: 0.2,
            place: 0.4,
            show: 0.6,
        };
        // 騎手レートが他 factor と等しい → 平均不変 → スコアは騎手なしと一致（減点なし）。
        let with_equal_jockey = HorseFactors {
            course_gate: Some(fs(base)),
            horse_surface: Some(fs(base)),
            horse_distance: Some(fs(base)),
            jockey_surface: Some(fs(base)),
            horse_track_condition: None,
            trainer_surface: None,
            recent_form: None,
        };
        let without_jockey = HorseFactors {
            course_gate: Some(fs(base)),
            horse_surface: Some(fs(base)),
            horse_distance: Some(fs(base)),
            jockey_surface: None,
            horse_track_condition: None,
            trainer_surface: None,
            recent_form: None,
        };
        let s_with = raw_score(&with_equal_jockey, |r| r.win, &EstimationConfig::default());
        let s_without = raw_score(&without_jockey, |r| r.win, &EstimationConfig::default());
        assert!(
            (s_with - s_without).abs() < 1e-10,
            "騎手なしが減点されている: with={s_with}, without={s_without}"
        );
        assert!((s_without - 0.2).abs() < 1e-10);

        // 強い騎手（高レート）は加点、弱い騎手（低レート）は減点として正しく効く。
        let strong = HorseFactors {
            jockey_surface: Some(fs(RateTriple {
                win: 0.5,
                place: 0.5,
                show: 0.5,
            })),
            ..with_equal_jockey.clone()
        };
        let weak = HorseFactors {
            jockey_surface: Some(fs(RateTriple::default())),
            ..with_equal_jockey
        };
        assert!(raw_score(&strong, |r| r.win, &EstimationConfig::default()) > s_without);
        assert!(raw_score(&weak, |r| r.win, &EstimationConfig::default()) < s_without);
    }

    /// 馬場状態項（#73）が欠落項で不当に減点されないこと（重み付き平均、ADR 0007 の流儀）。
    /// レートが全 factor で等しいなら馬場項の有無でスコアは変わらず、「平均からの差」としてのみ効く。
    #[test]
    fn track_condition_none_not_penalized() {
        let base = RateTriple {
            win: 0.2,
            place: 0.4,
            show: 0.6,
        };
        let with_equal_tc = HorseFactors {
            course_gate: Some(fs(base)),
            horse_surface: Some(fs(base)),
            horse_distance: Some(fs(base)),
            jockey_surface: None,
            horse_track_condition: Some(fs(base)),
            trainer_surface: None,
            recent_form: None,
        };
        let without_tc = HorseFactors {
            horse_track_condition: None,
            ..with_equal_tc.clone()
        };
        let s_with = raw_score(&with_equal_tc, |r| r.win, &EstimationConfig::default());
        let s_without = raw_score(&without_tc, |r| r.win, &EstimationConfig::default());
        assert!(
            (s_with - s_without).abs() < 1e-10,
            "馬場実績なしが減点されている: with={s_with}, without={s_without}"
        );
        assert!((s_without - 0.2).abs() < 1e-10);

        // 道悪巧者（高レート）は加点、苦手（低レート）は減点として正しく効く。
        let strong = HorseFactors {
            horse_track_condition: Some(fs(RateTriple {
                win: 0.5,
                place: 0.5,
                show: 0.5,
            })),
            ..with_equal_tc.clone()
        };
        let weak = HorseFactors {
            horse_track_condition: Some(fs(RateTriple::default())),
            ..with_equal_tc
        };
        assert!(raw_score(&strong, |r| r.win, &EstimationConfig::default()) > s_without);
        assert!(raw_score(&weak, |r| r.win, &EstimationConfig::default()) < s_without);
    }

    /// 馬場状態項を含む場合でも win ≤ place ≤ show の単調性が維持されること（#73）。
    #[test]
    fn track_condition_keeps_monotonicity_in_estimate() {
        let entries = vec![
            (
                make_entry(1, "ウマA"),
                HorseFactors {
                    course_gate: Some(fs(RateTriple {
                        win: 0.3,
                        place: 0.5,
                        show: 0.7,
                    })),
                    horse_surface: Some(fs(RateTriple {
                        win: 0.2,
                        place: 0.4,
                        show: 0.6,
                    })),
                    horse_distance: Some(fs(RateTriple {
                        win: 0.1,
                        place: 0.3,
                        show: 0.5,
                    })),
                    jockey_surface: None,
                    // win 偏重の不自然な馬場レートでも単調化が是正する。
                    horse_track_condition: Some(fs(RateTriple {
                        win: 0.9,
                        place: 0.1,
                        show: 0.1,
                    })),
                    trainer_surface: None,
                    recent_form: None,
                },
            ),
            (
                make_entry(2, "ウマB"),
                HorseFactors {
                    course_gate: Some(fs(RateTriple {
                        win: 0.1,
                        place: 0.2,
                        show: 0.3,
                    })),
                    horse_surface: Some(fs(RateTriple {
                        win: 0.1,
                        place: 0.2,
                        show: 0.3,
                    })),
                    horse_distance: Some(fs(RateTriple {
                        win: 0.1,
                        place: 0.2,
                        show: 0.3,
                    })),
                    jockey_surface: None,
                    horse_track_condition: None,
                    trainer_surface: None,
                    recent_form: None,
                },
            ),
        ];
        let probs = estimate_probabilities(&entries);
        for p in &probs {
            assert!(
                p.win_prob <= p.place_prob + 1e-10 && p.place_prob <= p.show_prob + 1e-10,
                "単調性違反: {} win={} place={} show={}",
                p.horse_name.value(),
                p.win_prob,
                p.place_prob,
                p.show_prob
            );
        }
    }

    /// 調教師項（#74）が欠落項で不当に減点されないこと（重み付き平均、ADR 0007 の流儀）。
    /// レートが全 factor で等しいなら調教師項の有無でスコアは変わらず、「平均からの差」としてのみ効く。
    #[test]
    fn trainer_absent_not_penalized() {
        let base = RateTriple {
            win: 0.2,
            place: 0.4,
            show: 0.6,
        };
        let with_equal_trainer = HorseFactors {
            course_gate: Some(fs(base)),
            horse_surface: Some(fs(base)),
            horse_distance: Some(fs(base)),
            jockey_surface: None,
            trainer_surface: Some(fs(base)),
            horse_track_condition: None,
            recent_form: None,
        };
        let without_trainer = HorseFactors {
            trainer_surface: None,
            ..with_equal_trainer.clone()
        };
        let s_with = raw_score(&with_equal_trainer, |r| r.win, &EstimationConfig::default());
        let s_without = raw_score(&without_trainer, |r| r.win, &EstimationConfig::default());
        assert!(
            (s_with - s_without).abs() < 1e-10,
            "調教師実績なしが減点されている: with={s_with}, without={s_without}"
        );
        assert!((s_without - 0.2).abs() < 1e-10);

        // 名伯楽（高レート）は加点、苦手（低レート）は減点として正しく効く。
        let strong = HorseFactors {
            trainer_surface: Some(fs(RateTriple {
                win: 0.5,
                place: 0.5,
                show: 0.5,
            })),
            ..with_equal_trainer.clone()
        };
        let weak = HorseFactors {
            trainer_surface: Some(fs(RateTriple::default())),
            ..with_equal_trainer
        };
        assert!(raw_score(&strong, |r| r.win, &EstimationConfig::default()) > s_without);
        assert!(raw_score(&weak, |r| r.win, &EstimationConfig::default()) < s_without);
    }

    // ---- ベイズ縮約（#75） ----

    fn shrink_cfg(m: f64) -> EstimationConfig {
        EstimationConfig {
            shrinkage: Some(ShrinkageConfig { pseudo_count: m }),
            recency: None,
        }
    }

    #[test]
    fn shrink_rate_endpoints_and_monotonic() {
        let prior = 0.1;
        let m = 10.0;
        // k=0（実績ゼロ相当）は完全に prior。
        assert!((shrink_rate(0.9, 0, prior, m) - prior).abs() < 1e-12);
        // k≫m は ≈ 生レート（縮約がほぼ効かない）。
        assert!((shrink_rate(0.9, 100_000, prior, m) - 0.9).abs() < 1e-3);
        // k=m なら生レートと prior のちょうど中点。
        assert!((shrink_rate(0.9, 10, prior, m) - (0.9 + prior) / 2.0).abs() < 1e-12);
        // starts が増えるほど生レートへ単調に近づく（prior より高いレートで単調増加）。
        let s1 = shrink_rate(0.9, 1, prior, m);
        let s5 = shrink_rate(0.9, 5, prior, m);
        let s20 = shrink_rate(0.9, 20, prior, m);
        assert!(prior < s1 && s1 < s5 && s5 < s20 && s20 < 0.9);
    }

    /// 少データ（starts 小）の高レート factor は縮約で prior 側へ強く引かれ、
    /// 同じレートでも大データ（starts 大）より低いスコアになる。
    #[test]
    fn shrinkage_pulls_low_sample_toward_prior() {
        let high_rate = RateTriple {
            win: 0.8,
            place: 0.8,
            show: 0.8,
        };
        let few = HorseFactors {
            course_gate: Some(FactorStat {
                rate: high_rate,
                starts: 1,
            }),
            horse_surface: None,
            horse_distance: None,
            jockey_surface: None,
            trainer_surface: None,
            horse_track_condition: None,
            recent_form: None,
        };
        let many = HorseFactors {
            course_gate: Some(FactorStat {
                rate: high_rate,
                starts: 200,
            }),
            ..few.clone()
        };
        let cfg = shrink_cfg(10.0);
        let s_few = raw_score(&few, |r| r.win, &cfg);
        let s_many = raw_score(&many, |r| r.win, &cfg);
        // prior(=1/14≈0.071) < 少データ < 多データ < 生レート(0.8)。
        assert!(PRIOR_RATE.win < s_few && s_few < s_many && s_many < 0.8);
        // 縮約 off では starts に依らず生レートのまま（挙動不変の確認）。
        let off = EstimationConfig::default();
        assert!((raw_score(&few, |r| r.win, &off) - 0.8).abs() < 1e-12);
        assert!((raw_score(&many, |r| r.win, &off) - 0.8).abs() < 1e-12);
    }

    /// 少データ馬が他の有力馬と同居しても、縮約により win_prob が 0 へ振り切れず
    /// 正値を保つ（ADR 0002 の `win_prob=0` 緩和）。
    #[test]
    fn shrinkage_keeps_low_sample_horse_above_zero() {
        // 1 頭は実績豊富で高レート、もう 1 頭は少データ（starts=1）で低レート。
        let strong = HorseFactors {
            course_gate: Some(FactorStat {
                rate: RateTriple {
                    win: 0.6,
                    place: 0.6,
                    show: 0.6,
                },
                starts: 100,
            }),
            horse_surface: None,
            horse_distance: None,
            jockey_surface: None,
            trainer_surface: None,
            horse_track_condition: None,
            recent_form: None,
        };
        let sparse = HorseFactors {
            course_gate: Some(FactorStat {
                rate: RateTriple::default(),
                starts: 1,
            }),
            ..strong.clone()
        };
        let entries = vec![
            (make_entry(1, "ウマ強"), strong),
            (make_entry(2, "ウマ薄"), sparse),
        ];
        let probs = estimate_probabilities_with_config(&entries, &shrink_cfg(10.0));
        let sparse_win = probs[1].win_prob;
        // 縮約により prior 方向へ持ち上がり、0 より大きい有限値になる。
        assert!(
            sparse_win > 0.0 && sparse_win.is_finite(),
            "sparse_win={sparse_win}"
        );
        // ただし強い馬よりは低い（順位は保つ）。
        assert!(probs[0].win_prob > sparse_win);
    }

    /// 本番 predict（`predict_race`）が使う `production()` の設定を固定する回帰ガード。
    /// 縮約 m を取り違えたり recency を誤って有効化すると CI で検知する（#75/ADR 0016）。
    #[test]
    fn production_config_is_shrinkage_m10_and_recency_off() {
        let c = EstimationConfig::production();
        assert_eq!(
            c.shrinkage.expect("production は縮約 on").pseudo_count,
            RECOMMENDED_SHRINKAGE_M
        );
        assert!((RECOMMENDED_SHRINKAGE_M - 10.0).abs() < 1e-12);
        assert!(
            c.recency.is_none(),
            "recency は backtest 評価で無効採用（ADR 0016）"
        );
    }

    // ---- リーセンシー重み付け（#75 Phase B） ----

    fn dc(date: NaiveDate, starts: u32, wins: u32) -> DatedCounts {
        DatedCounts {
            date,
            starts,
            wins,
            places: wins,
            shows: wins,
        }
    }

    #[test]
    fn recency_empty_or_all_future_is_none() {
        let as_of = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        assert!(apply_recency_weight(&[], as_of, 30.0).is_none());
        // as_of 当日・以降のみ → 全て無視され None（リーク防止）。
        let future = [
            dc(as_of, 1, 1),
            dc(NaiveDate::from_ymd_opt(2026, 6, 2).unwrap(), 1, 1),
        ];
        assert!(apply_recency_weight(&future, as_of, 30.0).is_none());
    }

    #[test]
    fn recency_weights_recent_runs_higher() {
        let as_of = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        // 直近 1 走で勝ち、半減期 1 つ前（30 日前）に負け。重みは直近が 2 倍なので
        // 重み付き勝率は単純平均 0.5 より高くなる。
        let runs = [
            dc(NaiveDate::from_ymd_opt(2026, 5, 2).unwrap(), 1, 0), // 30 日前: 着外
            dc(NaiveDate::from_ymd_opt(2026, 5, 31).unwrap(), 1, 1), // 1 日前: 勝ち
        ];
        let fs = apply_recency_weight(&runs, as_of, 30.0).expect("some");
        assert!(
            fs.rate.win > 0.5,
            "直近の勝ちが重く効くべき: {}",
            fs.rate.win
        );
        // 総出走数は時間重みを掛けない素の値。
        assert_eq!(fs.starts, 2);
    }

    #[test]
    fn recency_half_life_halves_weight() {
        let as_of = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        // half_life=30 日。直近(1 日前)勝ち1走 + 30 日前負け1走の重み比 ≈ 2:1。
        // 期待勝率 = w_recent / (w_recent + w_old)。
        let recent = (as_of - NaiveDate::from_ymd_opt(2026, 5, 31).unwrap()).num_days() as f64;
        let old = (as_of - NaiveDate::from_ymd_opt(2026, 5, 2).unwrap()).num_days() as f64;
        let w_recent = 0.5_f64.powf(recent / 30.0);
        let w_old = 0.5_f64.powf(old / 30.0);
        let expected = w_recent / (w_recent + w_old);
        let runs = [
            dc(NaiveDate::from_ymd_opt(2026, 5, 2).unwrap(), 1, 0),
            dc(NaiveDate::from_ymd_opt(2026, 5, 31).unwrap(), 1, 1),
        ];
        let fs = apply_recency_weight(&runs, as_of, 30.0).expect("some");
        assert!(
            (fs.rate.win - expected).abs() < 1e-12,
            "win={} expected={expected}",
            fs.rate.win
        );
    }

    fn prob(num: u32, win: f64, place: f64, show: f64) -> HorseProbability {
        HorseProbability {
            horse_num: HorseNum::try_from(num).unwrap(),
            horse_name: HorseName::try_from(format!("ウマ{num}")).unwrap(),
            win_prob: win,
            place_prob: place,
            show_prob: show,
        }
    }

    fn odds_map(pairs: &[(u32, f64)]) -> HashMap<HorseNum, f64> {
        pairs
            .iter()
            .map(|&(n, o)| (HorseNum::try_from(n).unwrap(), o))
            .collect()
    }

    #[test]
    fn blend_alpha_one_is_noop() {
        let probs = vec![prob(1, 0.6, 0.7, 0.8), prob(2, 0.4, 0.5, 0.6)];
        let out = blend_with_market_win(&probs, &odds_map(&[(1, 2.0), (2, 2.0)]), 1.0);
        for (a, b) in probs.iter().zip(&out) {
            approx(a.win_prob, b.win_prob);
            approx(a.place_prob, b.place_prob);
            approx(a.show_prob, b.show_prob);
        }
    }

    #[test]
    fn blend_empty_market_is_noop() {
        let probs = vec![prob(1, 0.6, 0.7, 0.8), prob(2, 0.4, 0.5, 0.6)];
        let out = blend_with_market_win(&probs, &HashMap::new(), 0.5);
        assert_eq!(out.len(), 2);
        approx(out[0].win_prob, 0.6);
        approx(out[1].win_prob, 0.4);
    }

    #[test]
    fn blend_removes_overround_and_mixes() {
        // モデル win = [0.5, 0.5]、オッズ [1.5, 3.0]。
        // implied = [0.6667, 0.3333], overround=1.0 → market_prob = [0.6667, 0.3333]
        // （このオッズは控除率0なので偶然 overround=1.0）。α=0.5 →
        // blended = [0.5*0.5+0.5*0.6667, 0.5*0.5+0.5*0.3333] = [0.5833, 0.4167]、合計1.0。
        let probs = vec![prob(1, 0.5, 0.6, 0.7), prob(2, 0.5, 0.6, 0.7)];
        let out = blend_with_market_win(&probs, &odds_map(&[(1, 1.5), (2, 3.0)]), 0.5);
        let m1 = (1.0 / 1.5) / (1.0 / 1.5 + 1.0 / 3.0);
        approx(out[0].win_prob, 0.5 * 0.5 + 0.5 * m1);
        approx(out[1].win_prob, 1.0 - out[0].win_prob);
        let total: f64 = out.iter().map(|p| p.win_prob).sum();
        approx(total, 1.0);
    }

    #[test]
    fn blend_normalizes_when_overround_above_one() {
        // 控除率あり: オッズ [1.5, 1.5] → implied=[0.667,0.667] overround=1.333 → market=[0.5,0.5]。
        let probs = vec![prob(1, 0.7, 0.8, 0.9), prob(2, 0.3, 0.4, 0.5)];
        let out = blend_with_market_win(&probs, &odds_map(&[(1, 1.5), (2, 1.5)]), 0.5);
        // market = [0.5,0.5]、blended=[0.6,0.4]、合計1.0。
        approx(out[0].win_prob, 0.6);
        approx(out[1].win_prob, 0.4);
        let total: f64 = out.iter().map(|p| p.win_prob).sum();
        approx(total, 1.0);
    }

    #[test]
    fn blend_keeps_monotonicity_and_unit_range() {
        // 市場が favorite の win を model.place 超へ押し上げても win ≤ place ≤ show を保つ。
        let probs = vec![prob(1, 0.4, 0.45, 0.5), prob(2, 0.6, 0.62, 0.7)];
        let out = blend_with_market_win(&probs, &odds_map(&[(1, 1.2), (2, 6.0)]), 0.2);
        for p in &out {
            assert!(
                p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
                "{p:?}"
            );
            assert!((0.0..=1.0).contains(&p.win_prob));
            assert!((0.0..=1.0).contains(&p.show_prob));
        }
    }

    #[test]
    fn blend_non_finite_alpha_is_noop() {
        // 非有限 α（NaN）は防御的に no-op（CLI で弾く前提だがドメイン単体でも保証）。
        let probs = vec![prob(1, 0.6, 0.7, 0.8), prob(2, 0.4, 0.5, 0.6)];
        let out = blend_with_market_win(&probs, &odds_map(&[(1, 2.0), (2, 2.0)]), f64::NAN);
        approx(out[0].win_prob, 0.6);
        approx(out[1].win_prob, 0.4);
    }

    #[test]
    fn blend_noop_when_all_odds_nonpositive() {
        // 全オッズが 0/負（型検証を経ない生 f64 経路の異常値）→ implied 空 → overround 0 → no-op。
        let probs = vec![prob(1, 0.6, 0.7, 0.8), prob(2, 0.4, 0.5, 0.6)];
        let out = blend_with_market_win(&probs, &odds_map(&[(1, 0.0), (2, -1.0)]), 0.5);
        approx(out[0].win_prob, 0.6);
        approx(out[1].win_prob, 0.4);
    }

    #[test]
    fn blend_partial_coverage_keeps_model_for_missing_and_renormalizes() {
        // 馬2 はオッズ無し → モデル値を保ちつつ全体は合計1.0へ再正規化。
        let probs = vec![prob(1, 0.5, 0.6, 0.7), prob(2, 0.5, 0.6, 0.7)];
        let out = blend_with_market_win(&probs, &odds_map(&[(1, 1.1)]), 0.5);
        let total: f64 = out.iter().map(|p| p.win_prob).sum();
        approx(total, 1.0);
        // 馬1 は超 favorite オッズなので blend で win が上がる。
        assert!(out[0].win_prob > out[1].win_prob);
    }

    #[test]
    fn recent_form_none_when_no_signals() {
        // 体重変化・人気・着順すべて欠損、かつ前走間隔も非正（同日）→ signal 無し → None。
        let prev = prev_result(None, None, None);
        assert!(recent_form_score(&prev, ymd(2026, 5, 1), ymd(2026, 5, 1), None).is_none());
    }

    #[test]
    fn recent_form_weight_change_smaller_is_better() {
        let d = ymd(2026, 5, 1);
        let pd = ymd(2026, 4, 1); // 30 日前（最適帯 1.0）
        // 体重変化のみで比較するため人気・着順は欠損。
        let stable = recent_form_score(&prev_result(Some(2), None, None), pd, d, None).unwrap();
        let swingy = recent_form_score(&prev_result(Some(18), None, None), pd, d, None).unwrap();
        assert!(stable > swingy, "stable={stable}, swingy={swingy}");
        // CAP(20kg) 超は体重 signal が 0。間隔 signal(1.0) との平均なので 0.5。
        let huge = recent_form_score(&prev_result(Some(40), None, None), pd, d, None).unwrap();
        assert!((huge - 0.5).abs() < 1e-9, "huge={huge}");
    }

    #[test]
    fn recent_form_popularity_gap() {
        let d = ymd(2026, 5, 1);
        let pd = ymd(2026, 4, 1);
        // 5 番人気で 2 着（人気以上に好走）→ 加点。1 番人気で 8 着（凡走）→ 減点。
        let over = recent_form_score(&prev_result(None, Some(5), Some(2)), pd, d, None).unwrap();
        let under = recent_form_score(&prev_result(None, Some(1), Some(8)), pd, d, None).unwrap();
        assert!(over > under, "over={over}, under={under}");
    }

    #[test]
    fn recent_form_interval_band() {
        // 最適帯(30日)=1.0、連闘(3日)=0.3、長休(200日)=0.5。間隔のみ（他欠損）。
        let base = ymd(2026, 5, 1);
        let optimal =
            recent_form_score(&prev_result(None, None, None), ymd(2026, 4, 1), base, None);
        let rento = recent_form_score(&prev_result(None, None, None), ymd(2026, 4, 28), base, None);
        let layoff = recent_form_score(
            &prev_result(None, None, None),
            ymd(2025, 10, 13),
            base,
            None,
        );
        assert!((optimal.unwrap() - 1.0).abs() < 1e-9);
        assert!((rento.unwrap() - 0.3).abs() < 1e-9);
        assert!((layoff.unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn recent_form_drops_popularity_signal_when_no_finish() {
        // 着順なし（中止・失格等で finishing_position = None）の前走は、人気が取れていても
        // 人気乖離 signal を落とし、体重変化(0.9: Δ=2)と間隔(1.0: 30日)のみで算出 → 平均 0.95。
        let d = ymd(2026, 5, 1);
        let pd = ymd(2026, 4, 1);
        let f = recent_form_score(&prev_result(Some(2), Some(3), None), pd, d, None).unwrap();
        let weight_sig = 1.0 - 2.0 / WEIGHT_CHANGE_CAP; // 0.9
        assert!((f - (weight_sig + 1.0) / 2.0).abs() < 1e-9, "form={f}");
    }

    #[test]
    fn recent_form_in_unit_range() {
        // 全 signal が揃ったケースでも [0,1]。
        let d = ymd(2026, 5, 1);
        let pd = ymd(2026, 4, 10);
        let f = recent_form_score(&prev_result(Some(-4), Some(3), Some(1)), pd, d, None).unwrap();
        assert!((0.0..=1.0).contains(&f), "form={f}");
    }

    #[test]
    fn parse_margin_keywords() {
        approx(parse_margin_lengths("ハナ").unwrap(), 0.05);
        approx(parse_margin_lengths("アタマ").unwrap(), 0.10);
        approx(parse_margin_lengths("クビ").unwrap(), 0.25);
        approx(parse_margin_lengths("同着").unwrap(), 0.0);
        approx(parse_margin_lengths("大差").unwrap(), MARGIN_CAP_LENGTHS);
    }

    #[test]
    fn parse_margin_fractions_and_decimals() {
        approx(parse_margin_lengths("1/2").unwrap(), 0.5);
        approx(parse_margin_lengths("3/4").unwrap(), 0.75);
        approx(parse_margin_lengths("1.1/4").unwrap(), 1.25); // 整数1 + 分数1/4
        approx(parse_margin_lengths("2.1/2").unwrap(), 2.5);
        approx(parse_margin_lengths("0.6").unwrap(), 0.6); // netkeiba 形式の小数
        approx(parse_margin_lengths("2").unwrap(), 2.0);
        approx(parse_margin_lengths(" 1.0 ").unwrap(), 1.0); // 前後空白を許容
    }

    #[test]
    fn parse_margin_invalid_is_none() {
        // 空・記号のみ・分母0・非数値・負値に加え、整数+分数の分母0（1.1/0）・分子非数値（abc/2）・
        // 整数部欠落の先頭ドット（.5/2）も None に倒す。
        for s in [
            "", "   ", "-", "1/0", "abc", "-1.0", "1.1/0", "abc/2", ".5/2", "-1/2",
        ] {
            assert!(parse_margin_lengths(s).is_none(), "expected None for {s:?}");
        }
    }

    #[test]
    fn margin_form_win_rewards_dominance() {
        // 圧勝(大差)=1.0、僅差勝ち(0.05馬身)≈0.5 をわずかに上回る。
        let dominant = margin_form(1, MARGIN_CAP_LENGTHS);
        let narrow = margin_form(1, 0.05);
        approx(dominant, 1.0);
        assert!(narrow > 0.5 && narrow < 0.55, "narrow={narrow}");
        assert!(dominant > narrow);
    }

    #[test]
    fn margin_form_loss_penalizes_blowout() {
        // 大敗(大差)=0.0、接戦負け(0.05馬身)≈0.5 をわずかに下回る。
        let blown = margin_form(5, MARGIN_CAP_LENGTHS);
        let close = margin_form(2, 0.05);
        approx(blown, 0.0);
        assert!(close < 0.5 && close > 0.45, "close={close}");
        assert!(blown < close);
    }

    #[test]
    fn recent_form_includes_margin_signal() {
        // 着差以外を欠損させ間隔(30日=1.0)＋着差のみで評価。圧勝(大差勝ち)が大敗を上回る。
        let d = ymd(2026, 5, 1);
        let pd = ymd(2026, 4, 1);
        let mut winner = prev_result(None, None, Some(1));
        winner.margin = Some("大差".to_string());
        let mut loser = prev_result(None, None, Some(10));
        loser.margin = Some("大差".to_string());
        let wf = recent_form_score(&winner, pd, d, None).unwrap();
        let lf = recent_form_score(&loser, pd, d, None).unwrap();
        approx(wf, 1.0); // (間隔1.0 + 着差1.0)/2
        approx(lf, 0.5); // (間隔1.0 + 着差0.0)/2
        assert!(wf > lf);
    }

    #[test]
    fn recent_form_drops_margin_signal_when_unparseable() {
        // 着差が解釈不能なら margin signal を落とし、間隔(30日)のみ → 1.0。
        let d = ymd(2026, 5, 1);
        let pd = ymd(2026, 4, 1);
        let mut prev = prev_result(None, None, Some(3));
        prev.margin = Some("???".to_string());
        approx(recent_form_score(&prev, pd, d, None).unwrap(), 1.0);
    }

    #[test]
    fn time_form_faster_is_higher() {
        // 標準より速い（タイム小）→ >0.5、遅い→ <0.5、同値→0.5。
        let std = 100.0;
        assert!(time_form(98.0, std) > 0.5);
        assert!(time_form(102.0, std) < 0.5);
        approx(time_form(100.0, std), 0.5);
    }

    #[test]
    fn time_form_saturates_at_cap() {
        // CAP(=5%)を超える偏差は 0/1 に飽和。標準非正は中立 0.5（防御）。
        let std = 100.0;
        approx(time_form(std * (1.0 - TIME_DEV_CAP), std), 1.0); // ちょうど CAP 速い → 1.0
        approx(time_form(std * (1.0 - 2.0 * TIME_DEV_CAP), std), 1.0); // CAP 超でも 1.0 にクランプ
        approx(time_form(std * (1.0 + 2.0 * TIME_DEV_CAP), std), 0.0); // CAP 超の遅さ → 0.0
        approx(time_form(95.0, 0.0), 0.5);
    }

    #[test]
    fn recent_form_includes_time_signal() {
        // タイム以外を欠損させ、間隔(30日=1.0)＋タイムのみで評価。標準より速い前走が遅い前走を上回る。
        let d = ymd(2026, 5, 1);
        let pd = ymd(2026, 4, 1);
        let std = Some(100.0);
        let mut fast = prev_result(None, None, None);
        fast.time_seconds = Some(TimeSeconds::try_from(98.0).unwrap());
        let mut slow = prev_result(None, None, None);
        slow.time_seconds = Some(TimeSeconds::try_from(102.0).unwrap());
        let ff = recent_form_score(&fast, pd, d, std).unwrap();
        let sf = recent_form_score(&slow, pd, d, std).unwrap();
        assert!(ff > sf, "fast={ff}, slow={sf}");
        // 標準タイム未整備（None）ならタイム signal は落ち、間隔のみ → 1.0。
        approx(recent_form_score(&fast, pd, d, None).unwrap(), 1.0);
    }

    #[test]
    fn recent_form_keeps_monotonicity_in_estimate() {
        // recent_form を持つ馬・持たない馬が混在しても単調性は保たれる。
        let mut f_with = zero_factors();
        f_with.course_gate = Some(fs(RateTriple {
            win: 0.3,
            place: 0.4,
            show: 0.5,
        }));
        f_with.recent_form = Some(0.9);
        let mut f_without = zero_factors();
        f_without.course_gate = Some(fs(RateTriple {
            win: 0.2,
            place: 0.3,
            show: 0.4,
        }));
        let entries = vec![
            (make_entry(1, "ウマA"), f_with),
            (make_entry(2, "ウマB"), f_without),
        ];
        let probs = estimate_probabilities(&entries);
        for p in &probs {
            assert!(p.win_prob <= p.place_prob && p.place_prob <= p.show_prob);
        }
    }
}
