//! 出馬表の各馬 factor から win/place/show 確率を推定するドメインロジック（#72/#75 ほか）。
//!
//! 関心事ごとにサブモジュールへ分割している。公開 API は本モジュールから re-export し、
//! `prediction::Foo` のパスを保つ（呼び出し側・`crate` ルートの re-export は不変）。
//!
//! - [`model`] — 値オブジェクト・データ構造（factor レート、馬の確率、前走、標準タイム）
//! - [`config`] — 推定の挙動切替（ベイズ縮約・リーセンシー）
//! - [`weights`] — 重み・キャップ・prior の定数群
//! - [`scoring`] — factor の重み付き採点と前走フォーム sub-signal
//! - [`recency`] — 日付付き成績の時間減衰集計
//! - [`estimate`] — レース内確率推定と市場オッズブレンド
//! - [`parse`] — 前走着差文字列のパース

mod config;
mod estimate;
mod model;
mod parse;
mod recency;
mod scoring;
mod weights;

#[cfg(test)]
mod tests;

pub use config::{EstimationConfig, RECOMMENDED_SHRINKAGE_M, RecencyConfig, ShrinkageConfig};
pub use estimate::{
    blend_with_market_win, estimate_probabilities, estimate_probabilities_with_config,
};
pub use model::{
    DatedCounts, FactorStat, HorseFactors, HorseProbability, JockeyFormRun, RateTriple, RecentRun,
    StandardTimes,
};
pub use recency::apply_recency_weight;
pub use scoring::{jockey_recent_form_score, recent_form_score, weight_factor};
