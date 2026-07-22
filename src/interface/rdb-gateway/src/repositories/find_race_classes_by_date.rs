use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{RaceClass, RaceId};
use sqlx::PgPool;

use crate::error::Result;

/// 指定日の全レースのレースクラスを `race_id → race_class` で返す（`race_cards` 由来、#459）。
///
/// 監視ループの G1 裏レース検出（`is_g1_ura`）用に、`load_slots` の per-race `find_race_card`
/// （N+1）を日付一括に置き換えるための取得（[`find_post_times_by_date`] と同方針）。
/// race_class 未保存（NULL）は SQL で除外する（PDF 経路・取得失敗・判定不能＝正常系。warn なし。
/// マップに現れない＝`None` として扱われ、per-card 経路の `card.race_class == None` と同義）。
/// race_id / race_class として解釈できない行のみ warn を出してマップから除外する（1 行破損で全体を
/// 落とさない縮退。単体取得の [`find_race_card`] が `Error::Data` で失敗させるのとは意図的な方針差）。
///
/// [`find_post_times_by_date`]: super::find_post_times_by_date::find_post_times_by_date
/// [`find_race_card`]: super::find_race_card::find_race_card
pub async fn find_race_classes_by_date(
    pool: &PgPool,
    date: NaiveDate,
) -> Result<HashMap<RaceId, RaceClass>> {
    let date_str = date.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT race_id, race_class
        FROM race_cards
        WHERE date = $1
          AND race_class IS NOT NULL
        "#,
    )
    .bind(&date_str)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|(race_id, race_class)| {
            let id = RaceId::try_from(race_id.as_str())
                .inspect_err(|e| {
                    tracing::warn!(race_id, error = %e, "race_id を解釈できず race_class 一覧から除外");
                })
                .ok()?;
            let class = RaceClass::try_from(race_class.as_str())
                .inspect_err(|e| {
                    tracing::warn!(race_id, race_class, error = %e, "race_class を解釈できず一覧から除外");
                })
                .ok()?;
            Some((id, class))
        })
        .collect())
}
