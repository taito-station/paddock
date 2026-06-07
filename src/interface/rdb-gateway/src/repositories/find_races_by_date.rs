use chrono::NaiveDate;
use paddock_domain::{Race, RaceId, Surface, TrackCondition, Venue, Weather};
use sqlx::SqlitePool;

use crate::error::Result;

#[derive(sqlx::FromRow)]
struct RaceRow {
    race_id: String,
    venue: String,
    round: i64,
    day: i64,
    race_num: i64,
    surface: String,
    distance: i64,
    track_condition: Option<String>,
    weather: Option<String>,
}

/// 指定日のレースを race_num 昇順で取得する。
///
/// 予想は「結果がまだ無い未来レース」が主目的のため、成績由来の `races` だけでなく
/// 出馬表由来の `race_cards` も対象にする。両者を race_id で UNION し、`races` に
/// 既にある（=結果取り込み済み）レースは track_condition / weather を持つそちらを優先、
/// `race_cards` 側は当該レースが `races` に無いときのみ採用する。
/// race_id は開催（年・回・場・日）を内包し開催日が一意なので、除外サブクエリは
/// 日付で絞らず race_id だけで判定する（万一 races 側の date がズレていても重複しない）。
///
/// 予想フェーズで使うため `results`（着順）は読み込まず空 Vec で返す。
/// `WHERE date = $1` で絞り込むため、各行の date は引数 `date` と一致する。
///
/// netkeiba 近走由来の合成レース（`source='netkeiba'`、過去日付の `nk-<id>`）は
/// 予想対象ではないため `source = 'pdf'` で除外する（混入すると予想候補として現れてしまう）。
pub async fn find_races_by_date(pool: &SqlitePool, date: NaiveDate) -> Result<Vec<Race>> {
    let date_str = date.format("%Y-%m-%d").to_string();

    let rows: Vec<RaceRow> = sqlx::query_as(
        r#"
        SELECT race_id, venue, round, day, race_num,
               surface, distance, track_condition, weather
        FROM races
        WHERE date = $1
          AND source = 'pdf'
        UNION ALL
        SELECT race_id, venue, round, day, race_num,
               surface, distance, NULL AS track_condition, NULL AS weather
        FROM race_cards
        WHERE date = $1
          AND NOT EXISTS (SELECT 1 FROM races WHERE races.race_id = race_cards.race_id)
        ORDER BY race_num ASC
        "#,
    )
    .bind(&date_str)
    .fetch_all(pool)
    .await?;

    let mut races = Vec::with_capacity(rows.len());
    for row in rows {
        let track_condition = row
            .track_condition
            .map(|s| TrackCondition::try_from(s.as_str()))
            .transpose()?;
        let weather = row
            .weather
            .map(|s| Weather::try_from(s.as_str()))
            .transpose()?;

        races.push(Race {
            race_id: RaceId::try_from(row.race_id.as_str())?,
            date,
            venue: Venue::try_from(row.venue.as_str())?,
            round: row.round as u32,
            day: row.day as u32,
            race_num: row.race_num as u32,
            surface: Surface::try_from(row.surface.as_str())?,
            distance: row.distance as u32,
            track_condition,
            weather,
            results: Vec::new(),
        });
    }

    Ok(races)
}
