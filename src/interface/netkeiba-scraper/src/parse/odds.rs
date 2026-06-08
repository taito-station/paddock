use paddock_domain::HorseNum;
use paddock_use_case::netkeiba_scraper::FetchedWinOdds;
use serde_json::Value;

use crate::error::{Error, Result};

/// 単勝オッズ API (`api_get_jra_odds.html?type=1`) の JSON から各馬の単勝オッズ・人気を取る。
///
/// 構造: `data.odds["1"]` が単勝。キー=馬番 2 桁ゼロ詰め、値=`[オッズ, "0.0", 人気]`。
/// オッズが `---.-` 等パース不能な行（レース前）はスキップする。確定前は空 Vec を返し得る。
pub fn parse_win_odds(json: &str) -> Result<Vec<FetchedWinOdds>> {
    let root: Value =
        serde_json::from_str(json).map_err(|e| Error::Parse(format!("invalid odds JSON: {e}")))?;

    // data.odds["1"] が単勝のオッズ表。未掲載（レース前）なら空 Vec で返す。
    let Some(win_map) = root
        .get("data")
        .and_then(|d| d.get("odds"))
        .and_then(|o| o.get("1"))
        .and_then(|w| w.as_object())
    else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for (key, value) in win_map {
        let Ok(num) = key.parse::<u32>() else {
            continue;
        };
        let Ok(horse_num) = HorseNum::try_from(num) else {
            continue;
        };
        let arr = match value.as_array() {
            Some(a) => a,
            None => continue,
        };
        // index0 = オッズ, index2 = 人気。レース前は "---.-" 等でパース不能 → スキップ。
        let Some(odds) = arr.first().and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok())
        else {
            continue;
        };
        let popularity = arr
            .get(2)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u32>().ok());

        out.push(FetchedWinOdds {
            horse_num,
            odds,
            popularity,
        });
    }

    // 馬番昇順で安定させる（JSON オブジェクトのキー順に依存しない）。
    out.sort_by_key(|w| w.horse_num.value());
    Ok(out)
}
