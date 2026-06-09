use paddock_domain::HorseNum;
use paddock_use_case::netkeiba_scraper::{FetchedOdds, FetchedPlaceOdds, FetchedWinOdds};
use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// オッズ API (`api_get_jra_odds.html?type=1`) の JSON から単勝・複勝を取る。
///
/// 構造: `data.odds["1"]` が単勝（値 `[オッズ, "0.0", 人気]`）、`data.odds["2"]` が複勝
/// （値 `[下限, 上限, 人気]`）。キー=馬番 2 桁ゼロ詰め。オッズが `---.-` 等パース不能な
/// 行（レース前）はスキップする。確定前は単勝・複勝とも空で返し得る。
pub fn parse_win_place_odds(json: &str) -> Result<FetchedOdds> {
    let root: Value =
        serde_json::from_str(json).map_err(|e| Error::Parse(format!("invalid odds JSON: {e}")))?;

    // 正常時は status="result"。それ以外（API エラー）はレース前の未掲載と区別して
    // エラーにする（未掲載は status="result" のまま odds が空で返るため別物）。
    if let Some(status) = root
        .get("status")
        .and_then(|s| s.as_str())
        .filter(|&s| s != "result")
    {
        return Err(Error::Parse(format!(
            "odds API が result 以外を返しました: status={status}"
        )));
    }

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

fn str_f64(v: Option<&Value>) -> Option<f64> {
    v.and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok())
}

fn str_u32(v: Option<&Value>) -> Option<u32> {
    v.and_then(|v| v.as_str()).and_then(|s| s.parse::<u32>().ok())
}
