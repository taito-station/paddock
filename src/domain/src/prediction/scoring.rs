//! factor の重み付き採点とスコア正規化、前走フォーム sub-signal（間隔・着差・タイム・斤量）。

use chrono::NaiveDate;

use super::config::EstimationConfig;
use super::model::JockeyFormRun;
use super::model::{FactorStat, HorseFactors, RateTriple};
use super::parse::parse_margin_lengths;
use super::weights::{
    COURSE_GATE_WEIGHT, DISTANCE_WEIGHT, FORM_WEIGHT, JOCKEY_RECENT_FORM_WEIGHT, JOCKEY_WEIGHT,
    MARGIN_CAP_LENGTHS, POP_GAP_K, PRIOR_RATE, SURFACE_WEIGHT, TIME_DEV_CAP,
    TRACK_CONDITION_WEIGHT, TRAINER_WEIGHT, WEIGHT_CARRIED_CAP_KG, WEIGHT_CARRIED_WEIGHT,
    WEIGHT_CHANGE_CAP,
};
use crate::horse_result::HorseResult;

/// ベイズ縮約: 出走数 `starts`(=k) の少ない factor のレートを prior へ寄せる（#75）。
/// `smoothed = (k·rate + m·prior) / (k + m)`。k≫m で ≈rate、k=0 で =prior、単調に補間する。
pub(crate) fn shrink_rate(rate: f64, starts: u32, prior: f64, pseudo_count: f64) -> f64 {
    let k = starts as f64;
    (k * rate + pseudo_count * prior) / (k + pseudo_count)
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
pub(crate) fn raw_score(
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
        let fw = config.recent_form_weight.unwrap_or(FORM_WEIGHT);
        weighted += fw * form;
        weight += fw;
    }
    if let Some(w) = factors.weight_carried {
        weighted += WEIGHT_CARRIED_WEIGHT * w;
        weight += WEIGHT_CARRIED_WEIGHT;
    }
    if let Some(jrf) = factors.jockey_recent_form {
        weighted += JOCKEY_RECENT_FORM_WEIGHT * jrf;
        weight += JOCKEY_RECENT_FORM_WEIGHT;
    }
    if weight == 0.0 {
        return 0.0;
    }
    weighted / weight
}

/// スコアをレース内合計が `target` になるよう正規化し、各値を確率として `[0, 1]` にクランプする。
/// 全スコアが 0（出走馬全員のスタッツ未蓄積）の場合は均等フォールバック `target / n`（上限 1.0）。
pub(crate) fn normalize_to_sum(scores: &[f64], target: f64) -> Vec<f64> {
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
    if let (Some(t), Some(std)) = (prev.time_seconds.map(|x| x.value()), standard_time)
        && t > 0.0
    {
        signals.push(time_form(t, std));
    }

    if signals.is_empty() {
        None
    } else {
        Some(signals.iter().sum::<f64>() / signals.len() as f64)
    }
}

/// 騎手の直近 N 走から「フォームシグナル」 [0,1]（0.5=中立）を返す（#221）。
/// 各走の signal = clamp(0.5 + (人気順位 − 着順) × POP_GAP_K, 0, 1) の平均値。
/// `finishing_position` / `popularity` いずれかが `None` の走は母数から除外する。
/// 有効な走が 0 件なら `None`（騎手未登録・近走なしと同じ扱い）。
pub fn jockey_recent_form_score(runs: &[JockeyFormRun]) -> Option<f64> {
    let mut total = 0.0f64;
    let mut count = 0u32;
    for run in runs {
        if let (Some(pos), Some(pop)) = (run.finishing_position, run.popularity) {
            total += (0.5 + (pop as f64 - pos as f64) * POP_GAP_K).clamp(0.0, 1.0);
            count += 1;
        }
    }
    (count > 0).then(|| total / count as f64)
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
pub(crate) fn margin_form(position: u32, margin_lengths: f64) -> f64 {
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
/// 標準タイムが非正のときは比が定義できないため中立 0.5 を返す（防御）。`prev_time > 0` は
/// 呼び出し側（`recent_form_score` の `t > 0.0` ガード）が保証する前提で、本関数は prev 側の
/// 非正を検査しない（0 秒以下の異常タイムは sub-signal を母数から落とす方が中立 0.5 を混ぜるより適切なため）。
pub(crate) fn time_form(prev_time: f64, standard_time: f64) -> f64 {
    if standard_time <= 0.0 {
        return 0.5;
    }
    let dev = (standard_time - prev_time) / standard_time;
    (0.5 + 0.5 * dev / TIME_DEV_CAP).clamp(0.0, 1.0)
}

/// 当該馬の斤量 `weight`[kg] とレース内の field 平均斤量 `field_mean`[kg] から「斤量のレース内相対」
/// シグナル `[0,1]`（0.5=中立）を作る（#135）。kg 差 `dev = weight - field_mean` を
/// `WEIGHT_CARRIED_CAP_KG` で飽和させて線形に写像する。向きは **平均より重い＝加点（>0.5）・軽い＝減点**。
/// 当初は「重い＝負担大で減点」を仮説に置いたが、backtest（main との before/after・両符号比較,
/// 2026-03-28〜05-31 / 144R）で逆符号（重い→加点）が的中率・回収率・Brier・LogLoss を全面的に改善し、
/// 減点符号は的中率を下げたため加点を採用（ADR 0009 追補）。別定/ハンデで実績馬ほど重い斤量を課される
/// 選択効果が「負担で遅くなる」効果を上回るため。`field_mean` 非正は防御として中立 0.5。
/// レース内相対の計算は use-case（`build_factors`）が field 平均を出して呼ぶため `pub`。
pub fn weight_factor(weight: f64, field_mean: f64) -> f64 {
    // field 平均が非正/非有限、または weight が非有限（NaN/inf）のときは比が定義できないため中立 0.5。
    // field_mean が NaN だと `NaN <= 0.0` を素通りし dev→NaN→clamp が NaN を返してレース全馬の確率を
    // 汚染する（normalize_to_sum の合計も NaN 化）ため、weight 側と対称に明示ガードする。
    if !field_mean.is_finite() || field_mean <= 0.0 || !weight.is_finite() {
        return 0.5;
    }
    let dev = weight - field_mean;
    (0.5 + 0.5 * dev / WEIGHT_CARRIED_CAP_KG).clamp(0.0, 1.0)
}
