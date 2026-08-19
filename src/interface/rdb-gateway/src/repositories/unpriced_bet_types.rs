//! 「この券種は netkeiba 上で未発売だと確認できた」観測の読み書き（#632）。
//!
//! read-through の cache-hit 判定が、欠けている券種を「一過性の取得失敗」と「発売されていない」に
//! 切り分けるための記録。番兵は払戻倍率ではないので `race_odds` には入れず（ADR 0086 決定 1/3）、
//! 専用表 `race_odds_unpriced_observations` に置く。

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use paddock_domain::{BetType, RaceId};
use paddock_use_case::repository::UnpricedObservation;
use sqlx::{PgPool, Row};

use crate::error::Result;

/// race_id の未発売観測を全件読み出す。TTL の解釈は use-case 層が持つ（gateway は時計を持たない）。
///
/// 未知の `bet_type` ラベルは読み飛ばす（読み出し側の「未知は読み飛ばす」規律。保存側の
/// 「未知は書かない」と対。CHECK 制約があるので実際には入り得ないが、手で入れた行や
/// 将来の語彙追加で predict が止まらないようにする）。`observed_at` が rfc3339 として
/// 解釈できない行も同様に読み飛ばす——**壊れた観測を「新しい観測」と誤読して再取得を
/// 止めるより、取り直すほうが安全側**だから。
pub async fn find_unpriced_bet_types(
    pool: &PgPool,
    race_id: &RaceId,
) -> Result<Vec<UnpricedObservation>> {
    let rows = sqlx::query(
        r#"
        SELECT
            bet_type,
            observed_at
        FROM race_odds_unpriced_observations
        WHERE race_id = $1
        "#,
    )
    .bind(race_id.value())
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let label: String = row.try_get("bet_type")?;
        let observed_at: String = row.try_get("observed_at")?;
        let Ok(bet_type) = BetType::try_from(label.as_str()) else {
            tracing::warn!(
                race_id = race_id.value(),
                bet_type = label,
                "未発売観測の未知 bet_type 行を読み飛ばした"
            );
            continue;
        };
        let Ok(observed_at) = DateTime::parse_from_rfc3339(&observed_at) else {
            tracing::warn!(
                race_id = race_id.value(),
                bet_type = label,
                observed_at,
                "未発売観測の observed_at を rfc3339 として解釈できず読み飛ばした"
            );
            continue;
        };
        out.push(UnpricedObservation {
            bet_type,
            observed_at: observed_at.with_timezone(&Utc),
        });
    }
    Ok(out)
}

/// 1 回のスクレイプ観測を記録する。
///
/// `unpriced` を UPSERT し、`priced`（今回オッズが取れた券種）のマークを DELETE する。
/// 両者は同一トランザクションで適用する——片方だけ反映された中間状態が残ると、
/// 「発売開始したのに未発売マークが生きている」窓ができるため。
pub async fn record_unpriced_bet_types(
    pool: &PgPool,
    race_id: &RaceId,
    unpriced: &BTreeSet<BetType>,
    priced: &BTreeSet<BetType>,
    observed_at: DateTime<Utc>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let observed_at = observed_at.to_rfc3339();

    // **DELETE を先に打つ**。同一レースを 2 プロセスが同時に観測し、片方が trio を unpriced・
    // もう片方が trio を priced と判断すると、「INSERT で自分の行をロック → DELETE で相手の行を
    // 待つ」交差でデッドロックしうる。全トランザクションで DELETE → INSERT の順に揃えると
    // ロック取得順が一致して交差が起きない（unpriced と priced は排他なので順序を変えても結果は同じ）。
    if !priced.is_empty() {
        let labels: Vec<String> = priced.iter().map(|b| b.to_string()).collect();
        sqlx::query(
            r#"
            DELETE FROM race_odds_unpriced_observations
            WHERE race_id = $1
              AND bet_type = ANY($2)
            "#,
        )
        .bind(race_id.value())
        .bind(&labels)
        .execute(&mut *tx)
        .await?;
    }

    for bet_type in unpriced {
        sqlx::query(
            r#"
            INSERT INTO race_odds_unpriced_observations
                (race_id, bet_type, observed_at)
            VALUES ($1, $2, $3)
            ON CONFLICT(race_id, bet_type) DO UPDATE SET
                observed_at = excluded.observed_at
            "#,
        )
        .bind(race_id.value())
        .bind(bet_type.to_string())
        .bind(&observed_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
