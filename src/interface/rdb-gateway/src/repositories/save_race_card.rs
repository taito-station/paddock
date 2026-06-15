use paddock_domain::RaceCard;
use sqlx::SqlitePool;

use crate::error::Result;

use super::sql::delete_absent_horse_nums;

pub async fn save_race_card(pool: &SqlitePool, card: &RaceCard) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO race_cards (race_id, date, venue, round, day, race_num, surface, distance)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT(race_id) DO UPDATE SET
            date     = excluded.date,
            venue    = excluded.venue,
            round    = excluded.round,
            day      = excluded.day,
            race_num = excluded.race_num,
            surface  = excluded.surface,
            distance = excluded.distance
        "#,
    )
    .bind(card.race_id.value())
    .bind(card.date.format("%Y-%m-%d").to_string())
    .bind(card.venue.as_jp())
    .bind(card.round as i64)
    .bind(card.day as i64)
    .bind(card.race_num as i64)
    .bind(card.surface.as_str())
    .bind(card.distance as i64)
    .execute(&mut *tx)
    .await?;

    // 破壊的な全消し DELETE はしない。fetch-card(netkeiba) と parse-entries(pdf) が同じ
    // race_id を書きうるため、全消しは一方の取り込みを消す危険がある。ON CONFLICT 更新に任せ、
    // 今回の出走集合に無い馬番だけを後段で掃除する。掃除は source 非依存なので、各経路は
    // その race の出走全頭（full field）を渡す前提（部分集合を書くと他経路ぶんを消しうる）。
    for entry in &card.entries {
        sqlx::query(
            r#"
            INSERT INTO horse_entries (race_id, gate_num, horse_num, horse_name, jockey, trainer, weight_carried)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT(race_id, horse_num) DO UPDATE SET
                gate_num   = excluded.gate_num,
                horse_name = excluded.horse_name,
                jockey     = excluded.jockey,
                -- trainer は netkeiba 経路のみが埋める（PDF 経路は NULL）。新値が NULL のときは
                -- 既存値を保持し、後続の PDF 取り込みが netkeiba の trainer を消さないようにする（#74）。
                trainer    = COALESCE(excluded.trainer, horse_entries.trainer),
                -- weight_carried も同様に netkeiba 経路のみが埋める。PDF 経路の NULL で上書きしない（#135）。
                weight_carried = COALESCE(excluded.weight_carried, horse_entries.weight_carried)
            "#,
        )
        .bind(card.race_id.value())
        .bind(entry.gate_num.value() as i64)
        .bind(entry.horse_num.value() as i64)
        .bind(entry.horse_name.value())
        .bind(entry.jockey.as_ref().map(|j| j.value().to_string()))
        .bind(entry.trainer.as_ref().map(|t| t.value().to_string()))
        .bind(entry.weight_carried)
        .execute(&mut *tx)
        .await?;
    }

    // 今回の出走集合に無い馬番（取消等で消えた行）だけを掃除する。
    let horse_nums: Vec<i64> = card
        .entries
        .iter()
        .map(|e| e.horse_num.value() as i64)
        .collect();
    delete_absent_horse_nums(&mut tx, "horse_entries", card.race_id.value(), &horse_nums).await?;

    tx.commit().await?;
    Ok(())
}
