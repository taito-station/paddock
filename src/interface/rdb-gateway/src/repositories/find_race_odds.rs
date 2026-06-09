use chrono::NaiveDate;
use paddock_domain::{
    HorseNum, OddsValue, OrderedPair, OrderedTriple, Pair, PlaceOdds, RaceId, RaceOdds, Triple,
};
use sqlx::SqlitePool;

use crate::error::{Error, Result};

#[derive(sqlx::FromRow)]
struct OddsRow {
    bet_type: String,
    combination_key: String,
    odds: f64,
    odds_high: Option<f64>,
}

/// `race_odds` の全券種行を読み出してドメイン [`RaceOdds`] に再構成する（#38）。
///
/// `as_of = Some(d)` のとき `date(fetched_at) <= d` のスナップショットに限定する
/// （backtest の当時オッズ参照、リーク防止）。`None` は時刻制約なし（predict の最新参照）。
/// `fetched_at` は常に UTC(RFC3339)で保存されるため、`date(fetched_at)` も UTC 日付で比較する。
/// いずれの券種の行も無ければ `None`。combination_key はドメインの `from_key()` でパースし、
/// 単勝・複勝の単一馬番キーは [`parse_horse_num`] で扱う。
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
        match row.bet_type.as_str() {
            "win" => {
                let horse_num = parse_horse_num(race_id, &row.combination_key)?;
                odds.win.insert(horse_num, OddsValue::try_from(row.odds)?);
            }
            "place" => {
                let horse_num = parse_horse_num(race_id, &row.combination_key)?;
                odds.place.insert(horse_num, parse_band(race_id, &row)?);
            }
            "quinella" => {
                let pair = parse_key(race_id, &row, Pair::from_key)?;
                odds.quinella.insert(pair, OddsValue::try_from(row.odds)?);
            }
            "wide" => {
                let pair = parse_key(race_id, &row, Pair::from_key)?;
                odds.wide.insert(pair, parse_band(race_id, &row)?);
            }
            "exacta" => {
                let pair = parse_key(race_id, &row, OrderedPair::from_key)?;
                odds.exacta.insert(pair, OddsValue::try_from(row.odds)?);
            }
            "trio" => {
                let triple = parse_key(race_id, &row, Triple::from_key)?;
                odds.trio.insert(triple, OddsValue::try_from(row.odds)?);
            }
            "trifecta" => {
                let triple = parse_key(race_id, &row, OrderedTriple::from_key)?;
                odds.trifecta.insert(triple, OddsValue::try_from(row.odds)?);
            }
            other => {
                return Err(Error::Data(format!(
                    "race_odds に想定外の bet_type '{other}' があります"
                )));
            }
        }
    }
    Ok(Some(odds))
}

/// `combination_key` を素の馬番（"1".."18"）としてパースする。win/place の単一馬番キー専用。
/// 組合せ券種の "1-2" 等のキーは各ドメイン型の `from_key`（[`parse_key`] 経由）で扱う。
fn parse_horse_num(race_id: &RaceId, key: &str) -> Result<HorseNum> {
    let num: u32 = key.parse().map_err(|_| {
        Error::Data(format!(
            "race_odds (race_id={}) の combination_key '{key}' は馬番として不正です",
            race_id.value()
        ))
    })?;
    Ok(HorseNum::try_from(num)?)
}

/// 組合せ券種の `combination_key` をドメイン型の `from_key` でパースする。保存側の不整合は
/// race_id/key 付きで [`Error::Data`] に包んで報告する。
fn parse_key<T>(
    race_id: &RaceId,
    row: &OddsRow,
    from_key: impl Fn(&str) -> paddock_domain::Result<T>,
) -> Result<T> {
    from_key(&row.combination_key).map_err(|e| {
        Error::Data(format!(
            "race_odds {} 行 (race_id={}) の combination_key '{}' が不正です: {e}",
            row.bet_type,
            race_id.value(),
            row.combination_key
        ))
    })
}

/// 幅 odds（複勝・ワイド）を復元する。`odds`=下限・`odds_high`=上限。上限欠落・low>high は
/// 保存側の不整合なので race_id/key 付きでエラーにする。
fn parse_band(race_id: &RaceId, row: &OddsRow) -> Result<PlaceOdds> {
    let high = row.odds_high.ok_or_else(|| {
        Error::Data(format!(
            "race_odds {} 行 (race_id={}, key={}) の odds_high が NULL です",
            row.bet_type,
            race_id.value(),
            row.combination_key
        ))
    })?;
    let low = OddsValue::try_from(row.odds)?;
    let high = OddsValue::try_from(high)?;
    PlaceOdds::try_from((low, high)).map_err(|e| {
        Error::Data(format!(
            "race_odds {} 行 (race_id={}, key={}) の幅 odds が不正です: {e}",
            row.bet_type,
            race_id.value(),
            row.combination_key
        ))
    })
}
