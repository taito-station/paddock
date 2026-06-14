use paddock_domain::OddsValue;
use paddock_use_case::RaceOddsRecord;
use sqlx::SqlitePool;

use crate::error::Result;

/// オッズ値として不正か。値域条件を手書きで複製せず `OddsValue` の不変条件（finite かつ >= 1.0）
/// を単一の真実源として委譲する。読み取り側 `find_race_odds` の skip 判定（`OddsValue::try_from`）
/// と境界が必ず一致する。
fn is_invalid_odds(v: f64) -> bool {
    OddsValue::try_from(v).is_err()
}

/// 1 レース分のオッズを 1 トランザクションで upsert する。
/// 主キー `(race_id, bet_type, combination_key)` で衝突した行は最新値で上書きする。
///
/// 値域違反行（odds < 1.0・非有限。netkeiba の未公開組合せ 0 埋めなど）は warn を残して INSERT
/// しない。`race_odds` に無効値を入れない DB 境界のガードで、読み取り側(find_race_odds)の skip と
/// 二重で predict セッションの全停止を防ぐ(#114)。netkeiba 経路は生 f64 を渡すためここで一元的に弾く。
///
/// ここで弾くのは値域違反のみ。band（複勝・ワイド）の構造的不整合（odds_high NULL・low>high）は
/// 保存側バグの早期検知のため意図的にガードせず、読み取り側で `Error` として顕在化させる
/// （find_race_odds::parse_band 参照。「保存できるが読めない」のは検知すべき不正状態のため許容）。
pub async fn save_race_odds(pool: &SqlitePool, record: &RaceOddsRecord) -> Result<()> {
    let mut tx = pool.begin().await?;

    let fetched_at = record.fetched_at.to_rfc3339();
    for row in &record.rows {
        if is_invalid_odds(row.odds) || row.odds_high.is_some_and(is_invalid_odds) {
            tracing::warn!(
                race_id = record.race_id.value(),
                bet_type = row.bet_type,
                key = row.combination_key,
                odds = row.odds,
                odds_high = row.odds_high,
                "race_odds の不正オッズ行を保存せずスキップした"
            );
            continue;
        }
        sqlx::query(
            r#"
            INSERT INTO race_odds
                (race_id, bet_type, combination_key, odds, odds_high, popularity, fetched_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT(race_id, bet_type, combination_key) DO UPDATE SET
                odds       = excluded.odds,
                odds_high  = excluded.odds_high,
                -- 人気はスクレイプ経路(predict)では取れず NULL になる。既存の人気付き値を
                -- NULL で潰さないよう、新値が NULL のときは既存値を残す（odds は常に最新で上書き）。
                popularity = COALESCE(excluded.popularity, race_odds.popularity),
                fetched_at = excluded.fetched_at
            "#,
        )
        .bind(record.race_id.value())
        .bind(&row.bet_type)
        .bind(&row.combination_key)
        .bind(row.odds)
        .bind(row.odds_high)
        .bind(row.popularity.map(|p| p as i64))
        .bind(&fetched_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
