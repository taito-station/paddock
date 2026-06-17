use chrono::NaiveDate;
use paddock_domain::{
    BetType, HorseNum, OddsValue, OrderedPair, OrderedTriple, Pair, PlaceOdds, RaceId, RaceOdds,
    Triple,
};
use sqlx::PgPool;

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
    pool: &PgPool,
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
            AND ($2 IS NULL OR substr(fetched_at, 1, 10) <= $2)
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
        // bet_type は `BetType`(Display=snake_case)で書かれる。未知ラベルの行は読み飛ばす:
        // SQL の bet_type フィルタを撤廃した(#38)ため、将来の券種追加を書く新版 → 旧版で読む
        // 過渡期でも predict/backtest 全体を止めない（撤廃前の「未知は無視」挙動を維持する）。
        //
        // 不正行の扱いは 2 種類に切り分ける:
        // - 値域違反（odds < 1.0・非有限。旧版スクレイパの 0 埋め残骸など）→ `parse_odds_value`/
        //   `parse_band` が warn を残して当該行のみ skip。1 レコードの不整合で予想全体を止めない(#114)。
        // - combination_key 不正 / band 構造不正（odds_high NULL・low>high）→ 下の parse_key/parse_band
        //   が `Error::Data` で停止させる（保存側バグの早期検知。skip すると黙って消えるため）。
        let Ok(bet_type) = BetType::try_from(row.bet_type.as_str()) else {
            // サイレントに落とすと「なぜ exotic が出ないか」の調査が困難なので debug で残す。
            tracing::debug!(
                race_id = race_id.value(),
                bet_type = row.bet_type,
                "race_odds の未知 bet_type 行を読み飛ばした"
            );
            continue;
        };
        match bet_type {
            BetType::Win => {
                let horse_num = parse_horse_num(race_id, &row.combination_key)?;
                let Some(v) = parse_odds_value(race_id, &row, row.odds) else {
                    continue;
                };
                odds.win.insert(horse_num, v);
            }
            BetType::Place => {
                let horse_num = parse_horse_num(race_id, &row.combination_key)?;
                let Some(band) = parse_band(race_id, &row)? else {
                    continue;
                };
                odds.place.insert(horse_num, band);
            }
            BetType::Quinella => {
                let pair = parse_key(race_id, &row, Pair::from_key)?;
                let Some(v) = parse_odds_value(race_id, &row, row.odds) else {
                    continue;
                };
                odds.quinella.insert(pair, v);
            }
            BetType::Wide => {
                let pair = parse_key(race_id, &row, Pair::from_key)?;
                let Some(band) = parse_band(race_id, &row)? else {
                    continue;
                };
                odds.wide.insert(pair, band);
            }
            BetType::Exacta => {
                let pair = parse_key(race_id, &row, OrderedPair::from_key)?;
                let Some(v) = parse_odds_value(race_id, &row, row.odds) else {
                    continue;
                };
                odds.exacta.insert(pair, v);
            }
            BetType::Trio => {
                let triple = parse_key(race_id, &row, Triple::from_key)?;
                let Some(v) = parse_odds_value(race_id, &row, row.odds) else {
                    continue;
                };
                odds.trio.insert(triple, v);
            }
            BetType::Trifecta => {
                let triple = parse_key(race_id, &row, OrderedTriple::from_key)?;
                let Some(v) = parse_odds_value(race_id, &row, row.odds) else {
                    continue;
                };
                odds.trifecta.insert(triple, v);
            }
        }
    }
    Ok(Some(odds))
}

/// 保存済みオッズ値を [`OddsValue`] へ変換する。値域違反（odds < 1.0・非有限。旧版スクレイパの
/// 未公開組合せ 0 埋め残骸など）はレース・セッション全体を止めず、race_id/券種/キー付きの warn を
/// 残して `None`（=その行を読み飛ばす）を返す(#114)。combination_key の不正（[`parse_key`]）とは
/// 切り分け、こちらは保存側に残った無効値への耐性。
fn parse_odds_value(race_id: &RaceId, row: &OddsRow, value: f64) -> Option<OddsValue> {
    match OddsValue::try_from(value) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(
                race_id = race_id.value(),
                bet_type = row.bet_type,
                key = row.combination_key,
                odds = value,
                "race_odds の不正オッズ行を読み飛ばした: {e}"
            );
            None
        }
    }
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

/// 幅 odds（複勝・ワイド）を復元する。`odds`=下限・`odds_high`=上限。
///
/// 値域違反（下限・上限が odds < 1.0・非有限。0 埋め残骸など）は [`parse_odds_value`] 経由で
/// race_id/key 付きの warn を残し `Ok(None)`（=その行を読み飛ばす）を返す(#114)。一方、上限欠落
/// （odds_high NULL）と low>high は保存側の構造的不整合なので従来どおり `Error`（stop）で早期検知する。
///
/// 評価順は「odds_high NULL（構造）→ low/high の値域 → low>high（構造）」。NULL を値域より先に
/// 見るため、`odds` が値域違反かつ `odds_high` も NULL の行は skip ではなく stop になる（構造不正の
/// 早期検知を優先）。実害は無い: 実在する 0 埋め残骸は scalar の三連単で band 券種ではないため、
/// band でこの組合せが起きるのは想定しない異常データであり、その場合は黙って消すより stop が安全。
fn parse_band(race_id: &RaceId, row: &OddsRow) -> Result<Option<PlaceOdds>> {
    let high = row.odds_high.ok_or_else(|| {
        Error::Data(format!(
            "race_odds {} 行 (race_id={}, key={}) の odds_high が NULL です",
            row.bet_type,
            race_id.value(),
            row.combination_key
        ))
    })?;
    // low を先に評価して早期 return することで、下限・上限の両方が値域違反でも warn は 1 行に抑える。
    let Some(low) = parse_odds_value(race_id, row, row.odds) else {
        return Ok(None);
    };
    let Some(high) = parse_odds_value(race_id, row, high) else {
        return Ok(None);
    };
    PlaceOdds::try_from((low, high)).map(Some).map_err(|e| {
        Error::Data(format!(
            "race_odds {} 行 (race_id={}, key={}) の幅 odds が不正です: {e}",
            row.bet_type,
            race_id.value(),
            row.combination_key
        ))
    })
}
