use chrono::NaiveDate;
use paddock_domain::{HorseNum, OddsValue, PlaceOdds, RaceId, RaceOdds};
use sqlx::SqlitePool;

use crate::error::{Error, Result};

#[derive(sqlx::FromRow)]
struct OddsRow {
    bet_type: String,
    combination_key: String,
    odds: f64,
    odds_high: Option<f64>,
}

/// `race_odds` の単勝・複勝行を読み出してドメイン [`RaceOdds`] に再構成する。
///
/// `as_of = Some(d)` のとき `date(fetched_at) <= d` のスナップショットに限定する
/// （backtest の当時オッズ参照、リーク防止）。`None` は時刻制約なし（predict の最新参照）。
/// `fetched_at` は常に UTC(RFC3339)で保存されるため、`date(fetched_at)` も UTC 日付で比較する。
/// 単勝・複勝いずれの行も無ければ `None`。返す `RaceOdds` は win/place のみ充填し、
/// 組合せ券種(quinella/exacta/trio/trifecta)のマップは常に空（#38 で別途対応）。
pub async fn find_race_odds(
    pool: &SqlitePool,
    race_id: &RaceId,
    as_of: Option<NaiveDate>,
) -> Result<Option<RaceOdds>> {
    // as_of は NULL 許容バインドで「制約なし」と「日付以前」を 1 クエリに統一する。
    // 主キー先頭 race_id で対象は 1 レース分（高々十数行）に絞られるため、`date(fetched_at)` が
    // インデックス非対応（sargable でない）でも実害はない。
    // backtest の as_of は JST 開催日 `race.date`、`fetched_at` は UTC 取得時刻。両者の TZ は一致
    // しないため日付境界は厳密でない: レース後の深夜取得（翌 UTC 日付）は as_of から漏れて当時オッズを
    // 取りこぼし得るし、当日レース時間帯の取得は同日付で通過する。fetch-card/predict はレース前に
    // 走らせる運用前提なので実害は小さく、粗いリーク防止として許容する。
    let as_of_str = as_of.map(|d| d.format("%Y-%m-%d").to_string());
    let rows: Vec<OddsRow> = sqlx::query_as(
        r#"
        SELECT bet_type, combination_key, odds, odds_high
        FROM race_odds
        WHERE race_id = $1
            AND bet_type IN ('win', 'place')
            AND ($2 IS NULL OR date(fetched_at) <= $2)
        "#,
    )
    .bind(race_id.value())
    .bind(as_of_str)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(None);
    }

    let mut odds = RaceOdds::empty(race_id.clone());
    for row in rows {
        let horse_num = parse_horse_num(race_id, &row.combination_key)?;
        match row.bet_type.as_str() {
            "win" => {
                odds.win.insert(horse_num, OddsValue::try_from(row.odds)?);
            }
            "place" => {
                // 複勝は幅 odds。odds=下限、odds_high=上限。上限欠落は保存側の不整合なのでエラーにする。
                let high = row.odds_high.ok_or_else(|| {
                    Error::Data(format!(
                        "race_odds place 行 (race_id={}, horse={}) の odds_high が NULL です",
                        race_id.value(),
                        horse_num.value()
                    ))
                })?;
                let low = OddsValue::try_from(row.odds)?;
                let high = OddsValue::try_from(high)?;
                // low > high 等の保存側不整合は odds_high NULL と同様に race_id/horse 付きで報告する。
                let band = PlaceOdds::try_from((low, high)).map_err(|e| {
                    Error::Data(format!(
                        "race_odds place 行 (race_id={}, horse={}) の複勝幅が不正です: {e}",
                        race_id.value(),
                        horse_num.value()
                    ))
                })?;
                odds.place.insert(horse_num, band);
            }
            // IN 句で絞っているため通常到達しない。将来 bet_type 追加時の取りこぼし防止に明示。
            other => {
                return Err(Error::Data(format!(
                    "race_odds に想定外の bet_type '{other}' があります"
                )));
            }
        }
    }
    Ok(Some(odds))
}

/// `combination_key` を素の馬番（"1".."18"）としてパースする。win/place の単一馬番キー専用で、
/// 組合せ券種(#38)の "1-2" 等のキーには別パーサが要る（本関数は `IN ('win','place')` で絞った後に呼ぶ）。
fn parse_horse_num(race_id: &RaceId, key: &str) -> Result<HorseNum> {
    let num: u32 = key.parse().map_err(|_| {
        Error::Data(format!(
            "race_odds (race_id={}) の combination_key '{key}' は馬番として不正です",
            race_id.value()
        ))
    })?;
    Ok(HorseNum::try_from(num)?)
}
