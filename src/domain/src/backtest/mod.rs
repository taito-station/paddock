//! 予想精度バックテストの指標集計（純粋ロジック・IO なし）。
//!
//! 各評価レースの「予測と実着順の突合結果」（[`RaceEvaluation`]）を受け取り、
//! 的中率（単勝・連対・複勝）・想定回収率・確率校正指標（Brier / LogLoss / reliability 曲線）を
//! [`BacktestReport`] に集計する。校正は単勝・連対・複勝の各確率について算出し、人気帯・頭数帯の
//! セグメント別にも出す。確率推定の再現やデータ取得は use-case 層が担い、本モジュールは集計のみを
//! 行う（設計書 `docs/specifications/backtest.md` 参照）。
//!
//! 関心事ごとにサブモジュールへ分割している。公開 API は本モジュールから re-export し、
//! `backtest::Foo` のパスを保つ（呼び出し側・`crate` ルートの re-export は不変）。
//!
//! - [`model`] — 値オブジェクト・データ構造（突合入力・校正指標・各セグメント・レポート）
//! - [`metrics`] — Brier / LogLoss 校正と reliability 曲線
//! - [`segments`] — 人気帯・頭数帯・馬場別のセグメント集計と分類バンド
//! - [`exotic`] — 買い目（券種）単位の校正・回収率集計
//! - [`evaluate`] — 評価レース集合からレポートを集計するトップレベル関数

mod evaluate;
mod exotic;
mod metrics;
mod model;
mod segments;

#[cfg(test)]
mod tests;

pub use evaluate::evaluate;
pub use exotic::exotic_segments;
pub use model::{
    BacktestReport, CalibrationMetrics, ExoticBet, ExoticSegment, FeatureRow, FieldSizeSegment,
    HorseOutcome, PopularitySegment, RaceEvaluation, ReliabilityBin, SurfaceSegment,
    Top3RankDistribution,
};

// テストが参照する crate 内部シンボル。外部公開はしない。
#[cfg(test)]
pub(crate) use metrics::reliability;
#[cfg(test)]
pub(crate) use segments::{
    FIELD_SIZE_BANDS, POPULARITY_BANDS, SURFACE_BANDS, field_size_band, popularity_band,
    surface_band,
};
