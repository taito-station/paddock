use paddock_domain::{HorseNum, OrderedPair, OrderedTriple, Pair, Triple};
use paddock_use_case::netkeiba_scraper::{
    FetchedComboOdds, FetchedOdds, FetchedPlaceOdds, FetchedWinOdds,
};
use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// オッズ API (`api_get_jra_odds.html?type=1`) の JSON から単勝・複勝を取る。
///
/// 構造: `data.odds["1"]` が単勝（値 `[オッズ, "0.0", 人気]`）、`data.odds["2"]` が複勝
/// （値 `[下限, 上限, 人気]`）。キー=馬番 2 桁ゼロ詰め。オッズが `---.-` 等パース不能な
/// 行（レース前）はスキップする。確定前は単勝・複勝とも空で返し得る。
pub fn parse_win_place_odds(json: &str) -> Result<FetchedOdds> {
    let root = parse_validated_root(json)?;

    // 未掲載（レース前）なら単勝・複勝とも空で返す。
    let odds = root.get("data").and_then(|d| d.get("odds"));
    let win = odds
        .and_then(|o| o.get("1"))
        .and_then(|w| w.as_object())
        .map(parse_win_map)
        .unwrap_or_default();
    let place = odds
        .and_then(|o| o.get("2"))
        .and_then(|p| p.as_object())
        .map(parse_place_map)
        .unwrap_or_default();

    Ok(FetchedOdds { win, place })
}

/// 馬連オッズ（`type=4`、`data.odds["4"]`）をパースする（#102）。キー=馬番 2 桁×2（無順序）、
/// 値 `[オッズ, "0.0", 人気]`。未確定行はスキップ。
pub fn parse_quinella_odds(json: &str) -> Result<Vec<FetchedComboOdds<Pair>>> {
    parse_combo_odds(json, "4", |key| {
        let (a, b) = parse_two(key)?;
        Pair::try_from((a, b)).ok()
    })
}

/// 馬単オッズ（`type=6`、`data.odds["6"]`）をパースする（#102）。キー=馬番 2 桁×2（順序あり）。
pub fn parse_exacta_odds(json: &str) -> Result<Vec<FetchedComboOdds<OrderedPair>>> {
    parse_combo_odds(json, "6", |key| {
        let (a, b) = parse_two(key)?;
        OrderedPair::try_from((a, b)).ok()
    })
}

/// 三連複オッズ（`type=7`、`data.odds["7"]`）をパースする（#102）。キー=馬番 2 桁×3（無順序）。
pub fn parse_trio_odds(json: &str) -> Result<Vec<FetchedComboOdds<Triple>>> {
    parse_combo_odds(json, "7", |key| {
        let (a, b, c) = parse_three(key)?;
        Triple::try_from((a, b, c)).ok()
    })
}

/// 三連単オッズ（`type=8`、`data.odds["8"]`）をパースする（#102）。キー=馬番 2 桁×3（順序あり）。
pub fn parse_trifecta_odds(json: &str) -> Result<Vec<FetchedComboOdds<OrderedTriple>>> {
    parse_combo_odds(json, "8", |key| {
        let (a, b, c) = parse_three(key)?;
        OrderedTriple::try_from((a, b, c)).ok()
    })
}

/// JSON をパースし status を検証して root `Value` を返す。単勝・複勝・組合せ券種で共通。
fn parse_validated_root(json: &str) -> Result<Value> {
    let root: Value =
        serde_json::from_str(json).map_err(|e| Error::Parse(format!("invalid odds JSON: {e}")))?;

    // オッズが正常に掲載されている status を受理する。
    //   - "result": 確定後。
    //   - "middle": 発走前の前売り中。全頭分のオッズがそろった正常な JSON が返る。
    // それ以外（"NG"=未掲載・対象外など）は API エラーとして弾く。
    const OK_STATUSES: [&str; 2] = ["result", "middle"];
    if let Some(status) = root
        .get("status")
        .and_then(|s| s.as_str())
        .filter(|&s| !OK_STATUSES.contains(&s))
    {
        return Err(Error::Parse(format!(
            "odds API が想定外の status を返しました: status={status}"
        )));
    }
    Ok(root)
}

/// 組合せ券種（馬連・馬単・三連複・三連単）の共通パーサ。`sub_key` は `data.odds` 下の券種キー
/// （type と同じ番号）、`build` は netkeiba の数字キー（"0407" 等）を組合せ型に変換する。
/// 値配列は index0=オッズ（カンマ区切りあり）, index2=人気。未確定（`---.-`）行はスキップ。
fn parse_combo_odds<K>(
    json: &str,
    sub_key: &str,
    build: impl Fn(&str) -> Option<K>,
) -> Result<Vec<FetchedComboOdds<K>>> {
    let root = parse_validated_root(json)?;
    let Some(map) = root
        .get("data")
        .and_then(|d| d.get("odds"))
        .and_then(|o| o.get(sub_key))
        .and_then(|m| m.as_object())
    else {
        // 未公開（レース前）は券種マップが無い。空で返す。
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for (key, value) in map {
        let Some(combination) = build(key) else {
            continue;
        };
        let Some(arr) = value.as_array() else {
            continue;
        };
        let Some(odds) = str_f64(arr.first()) else {
            continue;
        };
        out.push(FetchedComboOdds {
            combination,
            odds,
            popularity: str_u32(arr.get(2)),
        });
    }
    Ok(out)
}

/// `data.odds["1"]`（単勝表）をパース。値配列 index0=オッズ, index2=人気。
fn parse_win_map(map: &Map<String, Value>) -> Vec<FetchedWinOdds> {
    let mut out = Vec::new();
    for (key, value) in map {
        let Some(horse_num) = parse_horse_num(key) else {
            continue;
        };
        let Some(arr) = value.as_array() else {
            continue;
        };
        // レース前は "---.-" 等でパース不能 → スキップ。
        let Some(odds) = str_f64(arr.first()) else {
            continue;
        };
        out.push(FetchedWinOdds {
            horse_num,
            odds,
            popularity: str_u32(arr.get(2)),
        });
    }
    // 馬番昇順で安定させる（JSON オブジェクトのキー順に依存しない）。
    out.sort_by_key(|w| w.horse_num.value());
    out
}

/// `data.odds["2"]`（複勝表）をパース。値配列 index0=下限, index1=上限, index2=人気。
fn parse_place_map(map: &Map<String, Value>) -> Vec<FetchedPlaceOdds> {
    let mut out = Vec::new();
    for (key, value) in map {
        let Some(horse_num) = parse_horse_num(key) else {
            continue;
        };
        let Some(arr) = value.as_array() else {
            continue;
        };
        // 複勝は下限・上限の両端が必要。いずれか欠ける/未確定ならスキップ。
        let (Some(odds_low), Some(odds_high)) = (str_f64(arr.first()), str_f64(arr.get(1))) else {
            continue;
        };
        out.push(FetchedPlaceOdds {
            horse_num,
            odds_low,
            odds_high,
            popularity: str_u32(arr.get(2)),
        });
    }
    out.sort_by_key(|p| p.horse_num.value());
    out
}

fn parse_horse_num(key: &str) -> Option<HorseNum> {
    HorseNum::try_from(key.parse::<u32>().ok()?).ok()
}

/// 2 桁ゼロ詰めの馬番 1 つを `HorseNum` にする（組合せキーの分割用）。
fn horse_num_at(key: &str, range: std::ops::Range<usize>) -> Option<HorseNum> {
    parse_horse_num(key.get(range)?)
}

/// 組合せキー "AABB"（馬番 2 桁×2）を 2 頭の馬番に分割する。
fn parse_two(key: &str) -> Option<(HorseNum, HorseNum)> {
    if key.len() != 4 {
        return None;
    }
    Some((horse_num_at(key, 0..2)?, horse_num_at(key, 2..4)?))
}

/// 組合せキー "AABBCC"（馬番 2 桁×3）を 3 頭の馬番に分割する。
fn parse_three(key: &str) -> Option<(HorseNum, HorseNum, HorseNum)> {
    if key.len() != 6 {
        return None;
    }
    Some((
        horse_num_at(key, 0..2)?,
        horse_num_at(key, 2..4)?,
        horse_num_at(key, 4..6)?,
    ))
}

/// オッズ文字列を f64 にする。組合せ券種の高額オッズはカンマ区切り（例 `"1,141.1"`）の
/// ことがあるため除去してからパースする。`"---.-"` 等はパース不能で `None`。
fn str_f64(v: Option<&Value>) -> Option<f64> {
    v.and_then(|v| v.as_str())
        .and_then(|s| s.replace(',', "").parse::<f64>().ok())
}

fn str_u32(v: Option<&Value>) -> Option<u32> {
    v.and_then(|v| v.as_str()).and_then(|s| s.parse::<u32>().ok())
}
