pub mod backtest;
pub mod board;
pub mod predict;
pub mod race_card;
pub mod races_by_date;
pub mod recommend;
pub mod session;

/// 騎手直近フォーム特徴量（#221）で使う近走取得上限数。
/// predict/backtest 双方から `super::JOCKEY_RECENT_FORM_LIMIT` で参照する。
const JOCKEY_RECENT_FORM_LIMIT: u32 = 10;
