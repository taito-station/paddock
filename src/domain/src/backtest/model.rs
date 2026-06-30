//! バックテスト集計の値オブジェクト・データ構造（純粋な型と最小限の impl）。

use chrono::NaiveDate;

use crate::Surface;
use crate::prediction::HorseFactors;

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
    pub(crate) fn won(&self) -> bool {
        self.finishing_position == Some(1)
    }
    /// 2 着以内だったか（連対校正の実測 outcome）。
    pub(crate) fn placed(&self) -> bool {
        matches!(self.finishing_position, Some(p) if p <= 2)
    }
    /// 3 着以内だったか（複勝校正の実測 outcome）。
    pub(crate) fn showed(&self) -> bool {
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
    pub(crate) fn field_size(&self) -> usize {
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
    pub(crate) const ZERO: Self = Self {
        brier: 0.0,
        log_loss: 0.0,
    };
}

/// reliability 曲線の 1 ビン（予測確率帯ごとの平均予測 vs 実測率）。
/// 単勝・連対・複勝で再利用する（#258）。
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
    /// ビン内の実測率（その券種で的中した割合, `count == 0` のとき 0）。`count > 0` のビンで
    /// 完全校正なら `mean_predicted` に一致する。
    pub observed_rate: f64,
}

/// 頭数帯セグメント（レース単位の集計）。
#[derive(Debug, Clone, PartialEq)]
pub struct FieldSizeSegment {
    /// 頭数帯ラベル（[`FIELD_SIZE_BANDS`](super::segments::FIELD_SIZE_BANDS)）。
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
    /// 馬場ラベル（[`SURFACE_BANDS`](super::segments::SURFACE_BANDS)）。
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
    /// 人気帯ラベル（[`POPULARITY_BANDS`](super::segments::POPULARITY_BANDS)）。
    pub label: String,
    /// この帯のエントリ数。
    pub entries: u32,
    /// 平均予測単勝確率。
    pub mean_win_prob: f64,
    /// 実測勝率。校正が取れていれば `mean_win_prob` に近づく。
    pub observed_win_rate: f64,
    /// この帯の単勝校正。
    pub win_calibration: CalibrationMetrics,
    /// 平均予測連対確率（2 着以内, #258）。
    pub mean_place_prob: f64,
    /// 実測連対率。`observed_place_rate − mean_place_prob` が大きく正なら過小評価。
    pub observed_place_rate: f64,
    /// 平均予測複勝確率（3 着以内, #258）。複勝圏の人気薄過小評価を見る主指標。
    pub mean_show_prob: f64,
    /// 実測複勝率。人気薄帯で `observed_show_rate ≫ mean_show_prob` なら過小評価が確定。
    pub observed_show_rate: f64,
}

/// 3 着以内に入線した馬の「モデル複勝(show_prob)順位」分布（#258）。
///
/// 各レースで全出走馬を `show_prob` 降順に並べ、実際に 3 着以内へ来た馬がモデルで何位だったかを
/// 数える。`model_rank_7_plus` が大きいほど、モデルは複勝圏に飛び込む人気薄を順位下位に沈めており
/// 取りこぼしている（= 複勝圏の過小評価）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Top3RankDistribution {
    /// 突合できた 3 着以内入線馬の延べ頭数（分母）。
    pub finishers: u32,
    /// うちモデル show_prob 順位が 1〜3 位だった数（モデルが圏内予測できていた）。
    pub model_rank_1_3: u32,
    /// 4〜6 位だった数。
    pub model_rank_4_6: u32,
    /// 7 位以下だった数（モデルが大きく外した人気薄の複勝圏入線）。
    pub model_rank_7_plus: u32,
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
///
/// 解釈の注意（いずれも計測アーティファクトで真の過信/過小ではない）:
/// - 小頭数（7 頭以下）の複勝/ワイドは、`select_bets` の採用確率が `show_prob`（3 着以内）である一方、
///   的中定義は払戻圏の 2 着以内になるため `mean_predicted` が `hit_rate` を上回りやすい（定義差）。
///   `select_bets` 側の確率源を頭数で切り替える精緻化は follow-up（#122）。
/// - 同着を含むレースは上位着が先頭馬のみ採られるため一部の組合せ券種で的中を取りこぼし、
///   `hit_rate`／回収率が悲観側へわずかに振れうる。
#[derive(Debug, Clone, PartialEq)]
pub struct ExoticSegment {
    /// 券種ラベル（[`EXOTIC_BET_TYPES`](super::exotic::EXOTIC_BET_TYPES)）。
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

/// 学習型モデル評価ハーネス（#272 Phase A）の 1 出走馬分の特徴量＋ラベル行。`analyze backtest
/// --dump-features` 要求時のみ収集される。特徴量は本番 predict と同じ walk-forward（`races.date < D`）
/// で算出した [`HorseFactors`]（市場ブレンド・冪変換の前の生値）で、ラベルは確定着順・人気、value 検証用に
/// 当時市場の単勝オッズを併載する。欠落（`Option` の `None`）は TSV で空セルとして書き出し、`0` 埋めしない。
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureRow {
    /// レース ID（12 桁）。
    pub race_id: String,
    /// 開催日（walk-forward の as-of 境界＝この日の `races.date < date` 統計で算出）。
    pub date: NaiveDate,
    /// 馬番。
    pub horse_num: u32,
    /// 本番 predict と同経路で算出した素性（ブレンド・冪変換前）。`None` 項は欠落（母数除外）。
    pub factors: HorseFactors,
    /// 確定着順（ラベル）。着順なし（除外・失格・競走中止等）は `None`。
    pub finishing_position: Option<u32>,
    /// 当時市場の単勝オッズ。当時 race_odds スナップショット（as-of）を優先し、無ければ PDF 確定単勝で
    /// 代替する（backtest の `top_pick_odds` と同一ソース）。いずれも欠落なら `None`。value 検証の分母に使う。
    pub win_odds: Option<f64>,
    /// 人気（1 = 1 番人気）。`None` は不明。
    pub popularity: Option<u32>,
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
    /// 連対（2 着以内）確率の reliability 曲線（#258）。
    pub place_reliability: Vec<ReliabilityBin>,
    /// 複勝（3 着以内）確率の reliability 曲線（#258）。低予測ビンで実率が上振れ＝裾の過小評価。
    pub show_reliability: Vec<ReliabilityBin>,
    /// 3 着以内入線馬のモデル複勝順位分布（#258）。
    pub top3_rank_distribution: Top3RankDistribution,
    /// 頭数帯別の集計（データのある帯のみ、[`FIELD_SIZE_BANDS`](super::segments::FIELD_SIZE_BANDS) 順）。
    pub by_field_size: Vec<FieldSizeSegment>,
    /// 人気帯別の集計（データのある帯のみ、[`POPULARITY_BANDS`](super::segments::POPULARITY_BANDS) 順）。
    pub by_popularity: Vec<PopularitySegment>,
    /// 馬場（芝/ダート）別の集計（データのある馬場のみ、[`SURFACE_BANDS`](super::segments::SURFACE_BANDS) 順）。
    pub by_surface: Vec<SurfaceSegment>,
    /// 買い目（curated）の券種別 校正・回収率（#121）。`evaluate` では空で、exotic 評価を行う
    /// 呼び出し側（backtest interactor）が [`exotic_segments`](super::exotic::exotic_segments) の結果で埋める（買い目は
    /// `RaceEvaluation`＝馬単位の集計とは別の「買い目単位」のため `evaluate` には含めない）。
    pub by_exotic: Vec<ExoticSegment>,
    /// 学習型モデル評価ハーネス用の特徴量ダンプ（#272 Phase A）。`by_exotic` と同様 `evaluate` では
    /// `None` で、ダンプ要求時のみ backtest interactor が per-horse の [`FeatureRow`] を集めて埋める
    /// （集計ではなく生の特徴量＋ラベルのため別フィールド）。未要求時は `None` で既存挙動と不変。
    pub feature_dump: Option<Vec<FeatureRow>>,
}

impl BacktestReport {
    /// 評価レースが 0 件のときの空レポート（指標は 0 / 回収率は `None`）。
    pub(crate) fn empty() -> Self {
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
            place_reliability: Vec::new(),
            show_reliability: Vec::new(),
            top3_rank_distribution: Top3RankDistribution::default(),
            by_field_size: Vec::new(),
            by_popularity: Vec::new(),
            by_surface: Vec::new(),
            by_exotic: Vec::new(),
            feature_dump: None,
        }
    }
}
