//! 予測確率と確定オッズから EV プラスの買い目を選定するドメインロジック（#121 ほか）。
//!
//! 関心事ごとにサブモジュールへ分割している。公開 API は本モジュールから re-export し、
//! `betting::Foo` のパスを保つ（呼び出し側・`crate` ルートの re-export は不変）。
//!
//! - [`model`] — 値オブジェクト（設定・買い目・推奨・確定着順）
//! - [`harville`] — Harville モデルによる連系・順序系券種の的中確率
//! - [`kelly`] — Kelly 基準の賭け額分数
//! - [`select`] — EV プラスの買い目選定・並べ替え・curation
//! - [`hit`] — 確定着順に対する買い目の的中判定

mod harville;
mod hit;
mod kelly;
mod model;
mod select;

#[cfg(test)]
mod tests;

pub use hit::bet_hit;
pub use model::{BetCombination, BettingConfig, BettingRecommendation, Podium};
pub use select::select_bets;

// 収支シミュレータ（simulation）が `betting::harville_trifecta` の crate 内パスで参照するため、
// 公開はせず crate 内部 re-export でパスを保つ。
pub(crate) use harville::harville_trifecta;
