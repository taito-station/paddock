//! 予想精度バックテストの指標集計（純粋ロジック・IO なし）。
//!
//! 各評価レースの「予測と実着順の突合結果」（[`RaceEvaluation`]）を受け取り、
//! 的中率（単勝・連対・複勝）・想定回収率・確率校正指標（Brier / LogLoss / reliability 曲線）を
//! [`BacktestReport`] に集計する。校正は単勝・連対・複勝の各確率について算出し、人気帯・頭数帯の
//! セグメント別にも出す。確率推定の再現やデータ取得は use-case 層が担い、本モジュールは集計のみを
//! 行う（設計書 `docs/specifications/backtest.md` 参照）。

use std::collections::HashMap;

use crate::Surface;

/// LogLoss で `ln(0)` を避けるための確率クランプ幅。`p` を `[EPS, 1-EPS]` に収める。
const LOG_LOSS_EPS: f64 = 1e-15;

/// 1 レース 1 賭けの想定賭け金（円）。トップ選好馬の単勝に固定額を賭ける。
///
/// `odds` は単勝確定オッズ（払戻倍率）で、的中時の払戻は `odds × STAKE_PER_RACE`。JRA 実払戻の
/// 端数処理（100 円あたり 10 円未満切り捨て）は行わない理論値であり、実払戻とは厳密には一致しない。
const STAKE_PER_RACE: f64 = 100.0;

/// reliability 曲線の等幅ビン数（`[0,0.1) … [0.9,1.0]`）。
const RELIABILITY_BINS: usize = 10;

/// 人気帯セグメントのラベル（出力順）。`popularity_band` の戻り値と一致させる。
const POPULARITY_BANDS: [&str; 6] = [
    "1番人気",
    "2-3番人気",
    "4-6番人気",
    "7-9番人気",
    "10番人気以下",
    "人気不明",
];

/// 頭数帯セグメントのラベル（出力順）。`field_size_band` の戻り値と一致させる。
const FIELD_SIZE_BANDS: [&str; 4] = ["～9頭", "10-12頭", "13-15頭", "16頭以上"];

/// 馬場（芝/ダート）セグメントのラベル（出力順）。`surface_band` の戻り値と一致させる。
const SURFACE_BANDS: [&str; 2] = ["芝", "ダート"];

/// 買い目（curated）券種別セグメントの出力順（#121）。`BetCombination::type_label()` と一致させ、
/// select_bets の priority 順に準拠（馬連→馬単→三連複→単勝→複勝→三連単、ワイドは末尾）。
/// `wide` は現状 select_bets が生成しない（収支シミュレータ専用）ため exotic 集計では常に空＝
/// dead entry だが、将来ワイドを買い目に含めたときの取りこぼし防止と type_label 網羅のため残す。
const EXOTIC_BET_TYPES: [&str; 7] = [
    "quinella", "exacta", "trio", "win", "place", "trifecta", "wide",
];

/// 1 出走馬の予測確率と実着の突合（校正指標の純粋入力）。
#[derive(Debug, Clone)]
pub struct HorseOutcome {
    /// 単勝確率（1 着になる予測確率）。
    pub win_prob: f64,
    /// 連対確率（2 着以内になる予測確率）。
    pub place_prob: f64,
    /// 複勝確率（3 着以内になる予測確率）。
    pub show_prob: f64,
    /// 着順。`None` は着順なし（除外・失格・競走中止等）で全 outcome を非的中扱い。
    pub finishing_position: Option<u32>,
    /// 人気（1 = 1 番人気）。人気帯セグメント用。`None` は不明。
    pub popularity: Option<u32>,
}

impl HorseOutcome {
    /// 1 着だったか（単勝校正の実測 outcome）。
    fn won(&self) -> bool {
        self.finishing_position == Some(1)
    }
    /// 2 着以内だったか（連対校正の実測 outcome）。
    fn placed(&self) -> bool {
        matches!(self.finishing_position, Some(p) if p <= 2)
    }
    /// 3 着以内だったか（複勝校正の実測 outcome）。
    fn showed(&self) -> bool {
        matches!(self.finishing_position, Some(p) if p <= 3)
    }
}

/// 1 レース分の予測と実着の突合結果（集計の純粋入力）。
#[derive(Debug, Clone)]
pub struct RaceEvaluation {
    /// 全出走馬の予測確率と実着。校正指標・reliability・セグメント集計に使う。
    pub horses: Vec<HorseOutcome>,
    /// トップ選好馬（`win_prob` 最大、同値は馬番昇順）の着順。
    /// 除外・失格等で着順が無い場合は `None`（非的中扱い）。
    pub top_pick_position: Option<u32>,
    /// トップ選好馬の単勝確定オッズ。`None` なら回収率の母数外。
    pub top_pick_odds: Option<f64>,
    /// 馬場（芝/ダート）。馬場別セグメントの分類軸。
    pub surface: Surface,
}

impl RaceEvaluation {
    /// 出走頭数（頭数帯セグメントの分類軸）。
    fn field_size(&self) -> usize {
        self.horses.len()
    }
}

/// 確率校正指標（いずれも小さいほど良い）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationMetrics {
    /// Brier スコア（予測確率と実測 0/1 の二乗誤差平均）。
    pub brier: f64,
    /// 対数損失。
    pub log_loss: f64,
}

impl CalibrationMetrics {
    /// 入力が空のときの既定値。
    const ZERO: Self = Self {
        brier: 0.0,
        log_loss: 0.0,
    };
}

/// reliability 曲線の 1 ビン（予測確率帯ごとの平均予測 vs 実測勝率）。
#[derive(Debug, Clone, PartialEq)]
pub struct ReliabilityBin {
    /// ビン下限（含む）。
    pub lower: f64,
    /// ビン上限（最終ビンのみ 1.0 を含む）。
    pub upper: f64,
    /// ビンに入ったエントリ数。
    pub count: u32,
    /// ビン内の平均予測確率（`count == 0` のとき 0）。
    pub mean_predicted: f64,
    /// ビン内の実測勝率（`count == 0` のとき 0）。`count > 0` のビンで完全校正なら
    /// `mean_predicted` に一致する。
    pub observed_rate: f64,
}

/// 頭数帯セグメント（レース単位の集計）。
#[derive(Debug, Clone, PartialEq)]
pub struct FieldSizeSegment {
    /// 頭数帯ラベル（[`FIELD_SIZE_BANDS`]）。
    pub label: String,
    /// この帯のレース数。
    pub races: u32,
    /// 単勝的中率（トップ選好馬が 1 着）。
    pub win_hit_rate: f64,
    /// 連対的中率（トップ選好馬が 2 着以内）。
    pub place_hit_rate: f64,
    /// 複勝的中率（トップ選好馬が 3 着以内）。
    pub show_hit_rate: f64,
    /// この帯の全エントリでの単勝校正。
    pub win_calibration: CalibrationMetrics,
}

/// 馬場（芝/ダート）セグメント（レース単位の集計）。
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceSegment {
    /// 馬場ラベル（[`SURFACE_BANDS`]）。
    pub label: String,
    /// この馬場のレース数。
    pub races: u32,
    /// 単勝的中率（トップ選好馬が 1 着）。
    pub win_hit_rate: f64,
    /// 連対的中率（トップ選好馬が 2 着以内）。
    pub place_hit_rate: f64,
    /// 複勝的中率（トップ選好馬が 3 着以内）。
    pub show_hit_rate: f64,
    /// この馬場の全エントリでの単勝校正。
    pub win_calibration: CalibrationMetrics,
}

/// 人気帯セグメント（馬エントリ単位の集計）。
#[derive(Debug, Clone, PartialEq)]
pub struct PopularitySegment {
    /// 人気帯ラベル（[`POPULARITY_BANDS`]）。
    pub label: String,
    /// この帯のエントリ数。
    pub entries: u32,
    /// 平均予測単勝確率。
    pub mean_win_prob: f64,
    /// 実測勝率。校正が取れていれば `mean_win_prob` に近づく。
    pub observed_win_rate: f64,
    /// この帯の単勝校正。
    pub win_calibration: CalibrationMetrics,
}

/// 買い目（券種）単位の評価入力（#121）。`select_bets` の curated 推奨をそのまま 1 点 1 件で渡す。
#[derive(Debug, Clone)]
pub struct ExoticBet {
    /// 券種ラベル（`BetCombination::type_label()`: win/place/quinella/exacta/wide/trio/trifecta）。
    pub bet_type: &'static str,
    /// その買い目の予測的中確率（Harville 等で算出）。
    pub predicted_prob: f64,
    /// 確定着順で実的中したか。
    pub hit: bool,
    /// 払戻オッズ（倍率）。回収率 = Σ(的中時 odds) / 点数（賭け金一定）。
    /// 複勝（place）は確定前のオッズ幅の中央値 `(low+high)/2`（select_bets と同じ近似）で、
    /// 実払戻とは厳密には一致しない。賭け金は券種・点数によらず一定（1 点 1 単位）を仮定し、
    /// 軸流しや予算配分のような現実的ポートフォリオは含まない（#122 のスコープ）。
    pub odds: f64,
}

/// 券種別の買い目（curated）校正・回収率セグメント（#121）。過信なら `mean_predicted ≫ hit_rate`。
#[derive(Debug, Clone, PartialEq)]
pub struct ExoticSegment {
    /// 券種ラベル（[`EXOTIC_BET_TYPES`]）。
    pub label: String,
    /// 評価した買い目点数。
    pub bets: u32,
    /// 平均予測的中確率。
    pub mean_predicted: f64,
    /// 実的中率。
    pub hit_rate: f64,
    /// 予測確率の校正（Brier/LogLoss）。
    pub calibration: CalibrationMetrics,
    /// 投票あたり回収率（Σ 的中オッズ / 点数。賭け金一定なので的中オッズの平均）。
    pub payout_rate: f64,
}

/// バックテストの集計結果。
#[derive(Debug, Clone, PartialEq)]
pub struct BacktestReport {
    /// 突合できた評価レース数（的中率の母数）。
    pub races_evaluated: u32,
    /// 単勝的中率（トップ選好馬が 1 着）。
    pub win_hit_rate: f64,
    /// 連対的中率（トップ選好馬が 2 着以内）。
    pub place_hit_rate: f64,
    /// 複勝的中率（トップ選好馬が 3 着以内）。
    pub show_hit_rate: f64,
    /// 想定回収率（Σ payout / Σ stake）。オッズ取得レースが 0 件なら `None`。
    pub payout_rate: Option<f64>,
    /// 回収率の母数（オッズが取得できたレース数）。
    pub payout_races: u32,
    /// Brier スコア（win, 小さいほど良い）。単勝の校正だけは既存互換で平坦フィールドに保持し、
    /// 連対/複勝は `place_calibration`/`show_calibration` に持たせる。母数（全馬エントリ）0 のとき 0。
    pub brier: f64,
    /// 対数損失（win, 小さいほど良い）。母数 0 のとき 0。
    pub log_loss: f64,
    /// 連対（2 着以内）確率の校正。
    pub place_calibration: CalibrationMetrics,
    /// 複勝（3 着以内）確率の校正。
    pub show_calibration: CalibrationMetrics,
    /// 単勝確率の reliability 曲線（等幅 10 ビン、空ビンも含む）。
    pub win_reliability: Vec<ReliabilityBin>,
    /// 頭数帯別の集計（データのある帯のみ、[`FIELD_SIZE_BANDS`] 順）。
    pub by_field_size: Vec<FieldSizeSegment>,
    /// 人気帯別の集計（データのある帯のみ、[`POPULARITY_BANDS`] 順）。
    pub by_popularity: Vec<PopularitySegment>,
    /// 馬場（芝/ダート）別の集計（データのある馬場のみ、[`SURFACE_BANDS`] 順）。
    pub by_surface: Vec<SurfaceSegment>,
    /// 買い目（curated）の券種別 校正・回収率（#121）。`evaluate` では空で、exotic 評価を行う
    /// 呼び出し側（backtest interactor）が [`exotic_segments`] の結果で埋める（買い目は
    /// `RaceEvaluation`＝馬単位の集計とは別の「買い目単位」のため `evaluate` には含めない）。
    pub by_exotic: Vec<ExoticSegment>,
}

impl BacktestReport {
    /// 評価レースが 0 件のときの空レポート（指標は 0 / 回収率は `None`）。
    fn empty() -> Self {
        Self {
            races_evaluated: 0,
            win_hit_rate: 0.0,
            place_hit_rate: 0.0,
            show_hit_rate: 0.0,
            payout_rate: None,
            payout_races: 0,
            brier: 0.0,
            log_loss: 0.0,
            place_calibration: CalibrationMetrics::ZERO,
            show_calibration: CalibrationMetrics::ZERO,
            win_reliability: Vec::new(),
            by_field_size: Vec::new(),
            by_popularity: Vec::new(),
            by_surface: Vec::new(),
            by_exotic: Vec::new(),
        }
    }
}

/// `(予測確率, 実現したか)` のペア集合から Brier / LogLoss を算出する。空なら `ZERO`。
fn calibration(pairs: &[(f64, bool)]) -> CalibrationMetrics {
    if pairs.is_empty() {
        return CalibrationMetrics::ZERO;
    }
    let mut brier_sum = 0.0f64;
    let mut log_loss_sum = 0.0f64;
    for &(prob, hit) in pairs {
        let y = if hit { 1.0 } else { 0.0 };
        brier_sum += (prob - y).powi(2);
        // ε クランプで ln(0) の発散を防ぐ（スタッツ希薄で確率 0 の馬が実際に来るケース）。
        let p = prob.clamp(LOG_LOSS_EPS, 1.0 - LOG_LOSS_EPS);
        log_loss_sum += -(y * p.ln() + (1.0 - y) * (1.0 - p).ln());
    }
    let n = pairs.len() as f64;
    CalibrationMetrics {
        brier: brier_sum / n,
        log_loss: log_loss_sum / n,
    }
}

/// `(予測確率, 実現したか)` を等幅 `bins` ビンに分け、各ビンの平均予測と実測率を返す。
/// 確率は `[0,1]` にクランプし、上端 `1.0` は最終ビンに含める。空ビンも `count = 0` で返す。
fn reliability(pairs: &[(f64, bool)], bins: usize) -> Vec<ReliabilityBin> {
    debug_assert!(bins > 0, "reliability requires at least one bin");
    let width = 1.0 / bins as f64;
    let mut sum_pred = vec![0.0f64; bins];
    let mut hits = vec![0u32; bins];
    let mut counts = vec![0u32; bins];

    for &(prob, hit) in pairs {
        let p = prob.clamp(0.0, 1.0);
        // p == 1.0 を最終ビンへ。境界値（例 0.3）は浮動小数点誤差で隣接ビンに入りうるが、
        // reliability の概観用途では許容する。
        let idx = ((p / width) as usize).min(bins - 1);
        sum_pred[idx] += p;
        if hit {
            hits[idx] += 1;
        }
        counts[idx] += 1;
    }

    (0..bins)
        .map(|i| {
            let count = counts[i];
            let (mean_predicted, observed_rate) = if count > 0 {
                (sum_pred[i] / count as f64, hits[i] as f64 / count as f64)
            } else {
                (0.0, 0.0)
            };
            ReliabilityBin {
                lower: i as f64 * width,
                upper: (i + 1) as f64 * width,
                count,
                mean_predicted,
                observed_rate,
            }
        })
        .collect()
}

/// 人気を人気帯ラベルへ分類する。`None` は「人気不明」。
fn popularity_band(popularity: Option<u32>) -> &'static str {
    match popularity {
        Some(1) => "1番人気",
        Some(2..=3) => "2-3番人気",
        Some(4..=6) => "4-6番人気",
        Some(7..=9) => "7-9番人気",
        Some(_) => "10番人気以下",
        None => "人気不明",
    }
}

/// 出走頭数を頭数帯ラベルへ分類する。
fn field_size_band(field_size: usize) -> &'static str {
    match field_size {
        0..=9 => "～9頭",
        10..=12 => "10-12頭",
        13..=15 => "13-15頭",
        _ => "16頭以上",
    }
}

/// 馬場を馬場ラベルへ分類する。
fn surface_band(surface: Surface) -> &'static str {
    match surface {
        Surface::Turf => "芝",
        Surface::Dirt => "ダート",
    }
}

/// 人気帯別の集計（馬エントリ単位）。データのある帯のみ [`POPULARITY_BANDS`] 順に返す。
fn popularity_segments(races: &[RaceEvaluation]) -> Vec<PopularitySegment> {
    let mut buckets: HashMap<&'static str, Vec<&HorseOutcome>> = HashMap::new();
    for race in races {
        for h in &race.horses {
            buckets
                .entry(popularity_band(h.popularity))
                .or_default()
                .push(h);
        }
    }

    POPULARITY_BANDS
        .iter()
        .filter_map(|&label| {
            // バケットは `or_default().push()` でしか作られないため、キーがあれば必ず非空。
            let horses = buckets.get(label)?;
            let entries = horses.len() as u32;
            let pairs: Vec<(f64, bool)> = horses.iter().map(|h| (h.win_prob, h.won())).collect();
            let mean_win_prob = horses.iter().map(|h| h.win_prob).sum::<f64>() / entries as f64;
            let observed_win_rate =
                horses.iter().filter(|h| h.won()).count() as f64 / entries as f64;
            Some(PopularitySegment {
                label: label.to_string(),
                entries,
                mean_win_prob,
                observed_win_rate,
                win_calibration: calibration(&pairs),
            })
        })
        .collect()
}

/// 頭数帯別の集計（レース単位）。データのある帯のみ [`FIELD_SIZE_BANDS`] 順に返す。
fn field_size_segments(races: &[RaceEvaluation]) -> Vec<FieldSizeSegment> {
    let mut buckets: HashMap<&'static str, Vec<&RaceEvaluation>> = HashMap::new();
    for race in races {
        buckets
            .entry(field_size_band(race.field_size()))
            .or_default()
            .push(race);
    }

    FIELD_SIZE_BANDS
        .iter()
        .filter_map(|&label| {
            // バケットは `or_default().push()` でしか作られないため、キーがあれば必ず非空。
            let group = buckets.get(label)?;
            let races_n = group.len() as f64;
            let mut win_hits = 0u32;
            let mut place_hits = 0u32;
            let mut show_hits = 0u32;
            let mut pairs: Vec<(f64, bool)> = Vec::new();
            for race in group {
                if let Some(pos) = race.top_pick_position {
                    if pos == 1 {
                        win_hits += 1;
                    }
                    if pos <= 2 {
                        place_hits += 1;
                    }
                    if pos <= 3 {
                        show_hits += 1;
                    }
                }
                for h in &race.horses {
                    pairs.push((h.win_prob, h.won()));
                }
            }
            Some(FieldSizeSegment {
                label: label.to_string(),
                races: group.len() as u32,
                win_hit_rate: win_hits as f64 / races_n,
                place_hit_rate: place_hits as f64 / races_n,
                show_hit_rate: show_hits as f64 / races_n,
                win_calibration: calibration(&pairs),
            })
        })
        .collect()
}

/// 馬場（芝/ダート）別の集計（レース単位）。データのある馬場のみ [`SURFACE_BANDS`] 順に返す。
/// `field_size_segments` と同じ集計方式（トップ選好馬の着順で的中、全馬エントリで単勝校正）。
fn surface_segments(races: &[RaceEvaluation]) -> Vec<SurfaceSegment> {
    let mut buckets: HashMap<&'static str, Vec<&RaceEvaluation>> = HashMap::new();
    for race in races {
        buckets
            .entry(surface_band(race.surface))
            .or_default()
            .push(race);
    }

    SURFACE_BANDS
        .iter()
        .filter_map(|&label| {
            // バケットは `or_default().push()` でしか作られないため、キーがあれば必ず非空。
            let group = buckets.get(label)?;
            let races_n = group.len() as f64;
            let mut win_hits = 0u32;
            let mut place_hits = 0u32;
            let mut show_hits = 0u32;
            let mut pairs: Vec<(f64, bool)> = Vec::new();
            for race in group {
                if let Some(pos) = race.top_pick_position {
                    if pos == 1 {
                        win_hits += 1;
                    }
                    if pos <= 2 {
                        place_hits += 1;
                    }
                    if pos <= 3 {
                        show_hits += 1;
                    }
                }
                for h in &race.horses {
                    pairs.push((h.win_prob, h.won()));
                }
            }
            Some(SurfaceSegment {
                label: label.to_string(),
                races: group.len() as u32,
                win_hit_rate: win_hits as f64 / races_n,
                place_hit_rate: place_hits as f64 / races_n,
                show_hit_rate: show_hits as f64 / races_n,
                win_calibration: calibration(&pairs),
            })
        })
        .collect()
}

/// 評価レース集合から [`BacktestReport`] を集計する。
///
/// 的中率の母数は `races.len()`（突合できたレース）。トップ選好馬の着順が `None` の
/// レースは全的中率で非的中として数える。回収率は `top_pick_odds` がある レースのみを母数に、
/// トップ選好馬が 1 着なら `odds × STAKE_PER_RACE` を払戻として計上する。校正指標（Brier /
/// LogLoss）は単勝・連対・複勝それぞれの全馬エントリを母数に算出し、reliability 曲線は単勝確率に
/// ついて、人気帯・頭数帯・馬場(芝/ダート)別のセグメントも併せて出す。
pub fn evaluate(races: &[RaceEvaluation]) -> BacktestReport {
    if races.is_empty() {
        return BacktestReport::empty();
    }

    let n = races.len() as f64;
    let mut win_hits = 0u32;
    let mut place_hits = 0u32;
    let mut show_hits = 0u32;

    let mut payout_races = 0u32;
    let mut total_stake = 0.0f64;
    let mut total_payout = 0.0f64;

    // 全エントリの校正用ペア。
    let mut win_pairs: Vec<(f64, bool)> = Vec::new();
    let mut place_pairs: Vec<(f64, bool)> = Vec::new();
    let mut show_pairs: Vec<(f64, bool)> = Vec::new();

    for race in races {
        if let Some(pos) = race.top_pick_position {
            if pos == 1 {
                win_hits += 1;
            }
            if pos <= 2 {
                place_hits += 1;
            }
            if pos <= 3 {
                show_hits += 1;
            }
        }

        if let Some(odds) = race.top_pick_odds {
            payout_races += 1;
            total_stake += STAKE_PER_RACE;
            if race.top_pick_position == Some(1) {
                total_payout += odds * STAKE_PER_RACE;
            }
        }

        for h in &race.horses {
            win_pairs.push((h.win_prob, h.won()));
            place_pairs.push((h.place_prob, h.placed()));
            show_pairs.push((h.show_prob, h.showed()));
        }
    }

    let payout_rate = if payout_races > 0 {
        Some(total_payout / total_stake)
    } else {
        None
    };

    let win_calibration = calibration(&win_pairs);

    BacktestReport {
        races_evaluated: races.len() as u32,
        win_hit_rate: win_hits as f64 / n,
        place_hit_rate: place_hits as f64 / n,
        show_hit_rate: show_hits as f64 / n,
        payout_rate,
        payout_races,
        brier: win_calibration.brier,
        log_loss: win_calibration.log_loss,
        place_calibration: calibration(&place_pairs),
        show_calibration: calibration(&show_pairs),
        win_reliability: reliability(&win_pairs, RELIABILITY_BINS),
        by_field_size: field_size_segments(races),
        by_popularity: popularity_segments(races),
        by_surface: surface_segments(races),
        // 買い目（curated）の校正・回収率は買い目単位の別入力（exotic_segments）で埋める（#121）。
        by_exotic: Vec::new(),
    }
}

/// 買い目（curated 推奨）を券種別に集計する（#121）。データのある券種のみ [`EXOTIC_BET_TYPES`] 順。
/// 各券種で 平均予測確率・実的中率・校正(Brier/LogLoss)・投票あたり回収率を出す。
pub fn exotic_segments(bets: &[ExoticBet]) -> Vec<ExoticSegment> {
    let mut buckets: HashMap<&'static str, Vec<&ExoticBet>> = HashMap::new();
    for b in bets {
        buckets.entry(b.bet_type).or_default().push(b);
    }

    EXOTIC_BET_TYPES
        .iter()
        .filter_map(|&label| {
            let group = buckets.get(label)?;
            let n = group.len() as f64;
            let pairs: Vec<(f64, bool)> = group.iter().map(|b| (b.predicted_prob, b.hit)).collect();
            let mean_predicted = group.iter().map(|b| b.predicted_prob).sum::<f64>() / n;
            let hits = group.iter().filter(|b| b.hit).count();
            let payout: f64 = group.iter().filter(|b| b.hit).map(|b| b.odds).sum();
            Some(ExoticSegment {
                label: label.to_string(),
                bets: group.len() as u32,
                mean_predicted,
                hit_rate: hits as f64 / n,
                calibration: calibration(&pairs),
                payout_rate: payout / n,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
    }

    /// テスト用の馬 outcome。win/place/show 確率と着順・人気を与える。
    fn horse(win: f64, place: f64, show: f64, pos: Option<u32>, pop: Option<u32>) -> HorseOutcome {
        HorseOutcome {
            win_prob: win,
            place_prob: place,
            show_prob: show,
            finishing_position: pos,
            popularity: pop,
        }
    }

    /// win 確率と着順だけ指定する簡易版（place/show は win と同値、人気なし）。
    fn win_horse(win: f64, pos: Option<u32>) -> HorseOutcome {
        horse(win, win, win, pos, None)
    }

    #[test]
    fn empty_returns_zeroed_report() {
        let report = evaluate(&[]);
        assert_eq!(report, BacktestReport::empty());
        assert_eq!(report.races_evaluated, 0);
        assert!(report.payout_rate.is_none());
        assert!(report.win_reliability.is_empty());
        assert!(report.by_field_size.is_empty());
        assert!(report.by_popularity.is_empty());
    }

    #[test]
    fn two_races_known_values() {
        let races = vec![
            RaceEvaluation {
                horses: vec![win_horse(0.5, Some(1)), win_horse(0.5, Some(2))],
                top_pick_position: Some(1),
                top_pick_odds: Some(2.0),
                surface: Surface::Turf,
            },
            RaceEvaluation {
                // トップ選好(0.6)は 3 着、勝ったのは 0.4 の馬。
                horses: vec![win_horse(0.6, Some(3)), win_horse(0.4, Some(1))],
                top_pick_position: Some(3),
                top_pick_odds: Some(5.0),
                surface: Surface::Turf,
            },
        ];
        let r = evaluate(&races);

        assert_eq!(r.races_evaluated, 2);
        approx(r.win_hit_rate, 0.5); // race1 のみ 1 着
        approx(r.place_hit_rate, 0.5); // race1 のみ 2 着以内
        approx(r.show_hit_rate, 1.0); // race1(1着) + race2(3着)

        // 回収率: stake=200, payout=race1 のみ 2.0*100=200 → 1.0
        assert_eq!(r.payout_races, 2);
        approx(r.payout_rate.unwrap(), 1.0);

        // Brier(win) = (0.25+0.25+0.36+0.36)/4
        approx(r.brier, (0.25 + 0.25 + 0.36 + 0.36) / 4.0);

        // LogLoss(win) = (-ln0.5 -ln0.5 -ln0.4 -ln0.4)/4
        let expected_ll = (-(0.5f64).ln() - (0.5f64).ln() - (0.4f64).ln() - (0.4f64).ln()) / 4.0;
        approx(r.log_loss, expected_ll);
    }

    #[test]
    fn place_and_show_calibration_known_values() {
        // 1 レース・2 頭。place_prob/show_prob と着順から連対/複勝校正を検証する。
        // 馬A: 1 着 → placed=true, showed=true。馬B: 4 着 → placed=false, showed=false。
        let races = vec![RaceEvaluation {
            horses: vec![
                horse(0.5, 0.7, 0.8, Some(1), Some(1)),
                horse(0.5, 0.6, 0.7, Some(4), Some(2)),
            ],
            top_pick_position: Some(1),
            top_pick_odds: None,
            surface: Surface::Turf,
        }];
        let r = evaluate(&races);

        // place: (0.7,true),(0.6,false) → Brier=((0.3)^2+(0.6)^2)/2
        approx(
            r.place_calibration.brier,
            (0.3f64.powi(2) + 0.6f64.powi(2)) / 2.0,
        );
        let place_ll = (-(0.7f64).ln() - (1.0 - 0.6f64).ln()) / 2.0;
        approx(r.place_calibration.log_loss, place_ll);

        // show: (0.8,true),(0.7,false) → Brier=((0.2)^2+(0.7)^2)/2
        approx(
            r.show_calibration.brier,
            (0.2f64.powi(2) + 0.7f64.powi(2)) / 2.0,
        );
        let show_ll = (-(0.8f64).ln() - (1.0 - 0.7f64).ln()) / 2.0;
        approx(r.show_calibration.log_loss, show_ll);
    }

    #[test]
    fn payout_rate_none_when_no_odds() {
        let races = vec![RaceEvaluation {
            horses: vec![win_horse(0.7, Some(1)), win_horse(0.3, Some(2))],
            top_pick_position: Some(1),
            top_pick_odds: None,
            surface: Surface::Turf,
        }];
        let r = evaluate(&races);
        assert_eq!(r.payout_races, 0);
        assert!(r.payout_rate.is_none());
        approx(r.win_hit_rate, 1.0);
    }

    #[test]
    fn top_pick_none_position_counts_as_miss() {
        let races = vec![RaceEvaluation {
            horses: vec![win_horse(0.4, None), win_horse(0.6, None)],
            top_pick_position: None, // 除外・失格等
            top_pick_odds: Some(3.0),
            surface: Surface::Turf,
        }];
        let r = evaluate(&races);
        approx(r.win_hit_rate, 0.0);
        approx(r.place_hit_rate, 0.0);
        approx(r.show_hit_rate, 0.0);
        assert_eq!(r.payout_races, 1);
        approx(r.payout_rate.unwrap(), 0.0);
    }

    #[test]
    fn zero_prob_winner_keeps_log_loss_finite() {
        let races = vec![RaceEvaluation {
            horses: vec![win_horse(0.0, Some(1)), win_horse(1.0, Some(5))],
            top_pick_position: Some(5),
            top_pick_odds: None,
            surface: Surface::Turf,
        }];
        let r = evaluate(&races);
        assert!(r.log_loss.is_finite(), "log_loss must be finite");
        assert!(r.brier.is_finite());
        assert!(r.log_loss > 0.0);
    }

    #[test]
    fn hit_rates_respect_inclusion() {
        let races = vec![
            RaceEvaluation {
                horses: vec![win_horse(1.0, Some(1))],
                top_pick_position: Some(1),
                top_pick_odds: None,
                surface: Surface::Turf,
            },
            RaceEvaluation {
                horses: vec![win_horse(1.0, Some(2))],
                top_pick_position: Some(2),
                top_pick_odds: None,
                surface: Surface::Turf,
            },
            RaceEvaluation {
                horses: vec![win_horse(1.0, Some(3))],
                top_pick_position: Some(3),
                top_pick_odds: None,
                surface: Surface::Turf,
            },
        ];
        let r = evaluate(&races);
        assert!(r.win_hit_rate <= r.place_hit_rate);
        assert!(r.place_hit_rate <= r.show_hit_rate);
        approx(r.win_hit_rate, 1.0 / 3.0);
        approx(r.place_hit_rate, 2.0 / 3.0);
        approx(r.show_hit_rate, 1.0);
    }

    #[test]
    fn reliability_bins_split_and_aggregate() {
        let pairs = [
            (0.05, false),
            (0.0, false),
            (0.95, true),
            (0.95, true),
            (1.0, true), // 上端は最終ビンへ
        ];
        let bins = reliability(&pairs, 10);
        assert_eq!(bins.len(), 10);

        // bin0 = [0.0,0.1): 0.05 と 0.0 の 2 件、勝ち 0。
        approx(bins[0].lower, 0.0);
        approx(bins[0].upper, 0.1);
        assert_eq!(bins[0].count, 2);
        approx(bins[0].mean_predicted, 0.025);
        approx(bins[0].observed_rate, 0.0);

        // 中間ビンは空。
        assert_eq!(bins[5].count, 0);
        approx(bins[5].mean_predicted, 0.0);
        approx(bins[5].observed_rate, 0.0);

        // bin9 = [0.9,1.0]: 0.95,0.95,1.0 の 3 件、全勝。
        assert_eq!(bins[9].count, 3);
        approx(bins[9].mean_predicted, (0.95 + 0.95 + 1.0) / 3.0);
        approx(bins[9].observed_rate, 1.0);
    }

    #[test]
    fn band_classification_boundaries() {
        assert_eq!(popularity_band(Some(1)), "1番人気");
        assert_eq!(popularity_band(Some(3)), "2-3番人気");
        assert_eq!(popularity_band(Some(4)), "4-6番人気");
        assert_eq!(popularity_band(Some(9)), "7-9番人気");
        assert_eq!(popularity_band(Some(10)), "10番人気以下");
        assert_eq!(popularity_band(None), "人気不明");

        assert_eq!(field_size_band(9), "～9頭");
        assert_eq!(field_size_band(10), "10-12頭");
        assert_eq!(field_size_band(15), "13-15頭");
        assert_eq!(field_size_band(18), "16頭以上");
    }

    #[test]
    fn band_functions_only_emit_declared_labels() {
        // band 関数の戻り値は必ず出力順定義の定数配列に含まれること。片方だけ変更したときの
        // 同期ずれ（セグメントが無言でドロップされる）をコンパイル時でなくテストで検出する。
        for pop in [
            None,
            Some(0u32),
            Some(1),
            Some(3),
            Some(6),
            Some(9),
            Some(18),
            Some(100),
        ] {
            assert!(
                POPULARITY_BANDS.contains(&popularity_band(pop)),
                "popularity_band({pop:?}) が POPULARITY_BANDS に無い"
            );
        }
        for n in [0usize, 8, 9, 10, 12, 15, 16, 30] {
            assert!(
                FIELD_SIZE_BANDS.contains(&field_size_band(n)),
                "field_size_band({n}) が FIELD_SIZE_BANDS に無い"
            );
        }
        for s in [Surface::Turf, Surface::Dirt] {
            assert!(
                SURFACE_BANDS.contains(&surface_band(s)),
                "surface_band({s:?}) が SURFACE_BANDS に無い"
            );
        }
    }

    #[test]
    fn popularity_segments_group_entries_in_band_order() {
        let races = vec![RaceEvaluation {
            horses: vec![
                horse(0.5, 0.6, 0.7, Some(1), Some(1)), // 1番人気・勝ち
                horse(0.2, 0.3, 0.4, Some(3), Some(2)), // 2-3番人気・負け
                horse(0.1, 0.2, 0.3, Some(2), Some(3)), // 2-3番人気・負け
                horse(0.05, 0.1, 0.2, Some(5), None),   // 人気不明
            ],
            top_pick_position: Some(1),
            top_pick_odds: None,
            surface: Surface::Turf,
        }];
        let r = evaluate(&races);

        // 出力順は POPULARITY_BANDS 順、データのある帯のみ。
        let labels: Vec<&str> = r.by_popularity.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["1番人気", "2-3番人気", "人気不明"]);

        let fav = &r.by_popularity[0];
        assert_eq!(fav.entries, 1);
        approx(fav.mean_win_prob, 0.5);
        approx(fav.observed_win_rate, 1.0);

        let band23 = &r.by_popularity[1];
        assert_eq!(band23.entries, 2);
        approx(band23.mean_win_prob, (0.2 + 0.1) / 2.0);
        approx(band23.observed_win_rate, 0.0);
    }

    #[test]
    fn field_size_segments_group_races_in_band_order() {
        // 8 頭立て(～9頭)を 2 レース、14 頭立て(13-15頭)を 1 レース。
        let small = |pos: Option<u32>| RaceEvaluation {
            horses: (0..8).map(|_| win_horse(0.1, Some(2))).collect(),
            top_pick_position: pos,
            top_pick_odds: None,
            surface: Surface::Turf,
        };
        let large = RaceEvaluation {
            horses: (0..14).map(|_| win_horse(0.07, Some(2))).collect(),
            top_pick_position: Some(1),
            top_pick_odds: None,
            surface: Surface::Turf,
        };
        let races = vec![small(Some(1)), small(Some(5)), large];
        let r = evaluate(&races);

        let labels: Vec<&str> = r.by_field_size.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["～9頭", "13-15頭"]);

        let s = &r.by_field_size[0];
        assert_eq!(s.races, 2);
        approx(s.win_hit_rate, 0.5); // 1 着は 1 レースのみ

        let l = &r.by_field_size[1];
        assert_eq!(l.races, 1);
        approx(l.win_hit_rate, 1.0);
    }

    #[test]
    fn surface_segments_group_races_in_band_order() {
        // 芝 2 レース（うち本命1着は1つ）、ダート 1 レース（本命1着）。
        let race = |surface: Surface, pos: Option<u32>| RaceEvaluation {
            horses: vec![win_horse(0.5, pos), win_horse(0.5, Some(9))],
            top_pick_position: pos,
            top_pick_odds: None,
            surface,
        };
        let races = vec![
            race(Surface::Turf, Some(1)),
            race(Surface::Turf, Some(4)),
            race(Surface::Dirt, Some(1)),
        ];
        let r = evaluate(&races);

        // 出力順は SURFACE_BANDS 順（芝→ダート）、データのある馬場のみ。
        let labels: Vec<&str> = r.by_surface.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["芝", "ダート"]);

        let turf = &r.by_surface[0];
        assert_eq!(turf.races, 2);
        approx(turf.win_hit_rate, 0.5); // 芝 2 戦で本命1着は1つ
        approx(turf.show_hit_rate, 0.5); // 本命の着順は 1 着と 4 着

        let dirt = &r.by_surface[1];
        assert_eq!(dirt.races, 1);
        approx(dirt.win_hit_rate, 1.0);

        // 片側馬場のみの入力では、その馬場 1 要素だけが返る（データのある馬場のみ）。
        let dirt_only = evaluate(&[race(Surface::Dirt, Some(1))]);
        let dirt_labels: Vec<&str> = dirt_only
            .by_surface
            .iter()
            .map(|s| s.label.as_str())
            .collect();
        assert_eq!(dirt_labels, vec!["ダート"]);
    }

    #[test]
    fn exotic_segments_group_and_aggregate_by_type() {
        let bets = vec![
            ExoticBet {
                bet_type: "quinella",
                predicted_prob: 0.3,
                hit: true,
                odds: 5.0,
            },
            ExoticBet {
                bet_type: "quinella",
                predicted_prob: 0.2,
                hit: false,
                odds: 8.0,
            },
            ExoticBet {
                bet_type: "trifecta",
                predicted_prob: 0.05,
                hit: false,
                odds: 50.0,
            },
        ];
        let segs = exotic_segments(&bets);
        // EXOTIC_BET_TYPES 順（quinella→…→trifecta）、データのある券種のみ。
        let labels: Vec<&str> = segs.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["quinella", "trifecta"]);

        let q = &segs[0];
        assert_eq!(q.bets, 2);
        approx(q.mean_predicted, 0.25);
        approx(q.hit_rate, 0.5);
        approx(q.payout_rate, 5.0 / 2.0); // 的中 1 点(odds5.0) / 2 点

        let t = &segs[1];
        assert_eq!(t.bets, 1);
        approx(t.hit_rate, 0.0);
        approx(t.payout_rate, 0.0);
    }

    #[test]
    fn exotic_payout_rate_sums_all_hits_over_total_bets() {
        // 同一券種で 2 点的中（賭け金一定前提）。回収率 = (的中オッズの和) / 総点数。
        let bets = vec![
            ExoticBet {
                bet_type: "win",
                predicted_prob: 0.5,
                hit: true,
                odds: 2.0,
            },
            ExoticBet {
                bet_type: "win",
                predicted_prob: 0.4,
                hit: true,
                odds: 3.0,
            },
            ExoticBet {
                bet_type: "win",
                predicted_prob: 0.3,
                hit: false,
                odds: 4.0,
            },
        ];
        let segs = exotic_segments(&bets);
        assert_eq!(segs.len(), 1);
        let w = &segs[0];
        assert_eq!(w.bets, 3);
        approx(w.hit_rate, 2.0 / 3.0);
        // (2.0 + 3.0) / 3 点 = 5/3。1 点でも複数的中でも分母は総点数。
        approx(w.payout_rate, 5.0 / 3.0);
    }
}
