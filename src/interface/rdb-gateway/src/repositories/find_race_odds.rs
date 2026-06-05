use paddock_domain::{RaceId, RaceOdds};
use sqlx::SqlitePool;

use crate::error::Result;

/// race_id に対応するオッズを返す。
///
/// 現時点では `race_odds` テーブルが存在しないため常に `None` を返す。
/// オッズの永続化（テーブル追加 + スクレイパー保存）は別 Issue のスコープ。
/// 設計: `docs/specifications/predict-session.md` の「オッズ永続化のスコープ外注意」を参照。
/// テーブル追加時にこの関数を SELECT 実装へ差し替える（シグネチャは将来の実装のため維持）。
pub async fn find_race_odds(_pool: &SqlitePool, _race_id: &RaceId) -> Result<Option<RaceOdds>> {
    Ok(None)
}
