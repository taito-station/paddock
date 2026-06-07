use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseName, HorseNum, JockeyName, ResultStatus, Surface,
    TimeSeconds, TrackCondition,
};
use paddock_use_case::HorsePastRun;
use scraper::{ElementRef, Html, Selector};

use super::{cell_text, round_day_racenum, venue_from_race_id};
use crate::error::{Error, Result};

// db_h_race_results テーブルの列インデックス（33 列。disp_none の隠し列も含め固定順）。
const COL_DATE: usize = 0;
const COL_RACE_NAME: usize = 4;
const COL_GATE: usize = 7;
const COL_HORSE_NUM: usize = 8;
const COL_ODDS: usize = 9;
const COL_POPULARITY: usize = 10;
const COL_FINISH: usize = 11;
const COL_JOCKEY: usize = 12;
const COL_WEIGHT_CARRIED: usize = 13;
const COL_DISTANCE: usize = 14;
const COL_TRACK_COND: usize = 16;
const COL_TIME: usize = 18;
const COL_MARGIN: usize = 19;
const COL_HORSE_WEIGHT: usize = 28;
const MIN_CELLS: usize = 29;

/// 馬個別成績ページ (`horse/result/<id>/`) のHTMLから JRA 平地の近走を抽出する。
///
/// 障害・地方・海外の行はスキップする（race_id リンクが無い=海外、場コードが JRA 外=地方、
/// 距離が `障…`=障害）。競走中止（着順 `中`）は馬番等が揃うため着順なし＋`status` 付きで
/// 保持するが、出走取消・除外で馬番/枠/距離など必須セルを欠く行はスキップする。
///
/// 列レイアウト変更などで全データ行が落ちると無言で空 Vec になり原因が見えにくいため、
/// データ行があったのに 1 件も抽出できなかった場合は warn を出す。
pub fn parse_horse_history(html: &str) -> Result<Vec<HorsePastRun>> {
    let doc = Html::parse_document(html);
    let table = doc
        .select(&sel("table.db_h_race_results")?)
        .next()
        .ok_or_else(|| Error::Parse("results table not found".to_string()))?;

    let horse_name = table
        .value()
        .attr("summary")
        .and_then(|s| s.strip_suffix("の競走戦績"))
        .and_then(cell_text)
        .and_then(|n| HorseName::try_from(n).ok())
        .ok_or_else(|| Error::Parse("horse name not found in table summary".to_string()))?;

    let tr_sel = sel("tr")?;
    let td_sel = sel("td")?;
    let a_race_sel = sel("a[href*=\"/race/\"]")?;

    let mut runs = Vec::new();
    let mut data_rows = 0usize;
    for row in table.select(&tr_sel) {
        let cells: Vec<ElementRef> = row.select(&td_sel).collect();
        if cells.len() < MIN_CELLS {
            continue; // ヘッダ行（th のみ）や壊れた行
        }
        data_rows += 1;
        if let Some(run) = parse_row(&cells, &a_race_sel, &horse_name) {
            runs.push(run);
        }
    }
    if data_rows > 0 && runs.is_empty() {
        tracing::warn!(
            horse = %horse_name,
            data_rows,
            "no rows extracted from horse history (列レイアウト変更の可能性)"
        );
    }
    Ok(runs)
}

fn parse_row(
    cells: &[ElementRef],
    a_race_sel: &Selector,
    horse_name: &HorseName,
) -> Option<HorsePastRun> {
    // レース名セルの /race/<id>/ リンクから netkeiba race_id を取得。
    // リンクが無い行は海外成績などで JRA race_id を持たないためスキップ。
    let race_id = cells[COL_RACE_NAME]
        .select(a_race_sel)
        .next()
        .and_then(|a| a.value().attr("href"))
        .and_then(extract_race_id)?;
    let venue = venue_from_race_id(&race_id)?; // JRA 外(地方)はスキップ
    let (round, day, race_num) = round_day_racenum(&race_id)?;

    let text = |i: usize| {
        cells
            .get(i)
            .and_then(|c| cell_text(&c.text().collect::<String>()))
    };

    let date = NaiveDate::parse_from_str(text(COL_DATE)?.as_str(), "%Y/%m/%d").ok()?;
    let (surface, distance) = parse_surface_distance(text(COL_DISTANCE)?.as_str())?; // 障害はスキップ
    let gate_num = text(COL_GATE)?
        .parse::<u32>()
        .ok()
        .and_then(|n| GateNum::try_from(n).ok())?;
    let horse_num = text(COL_HORSE_NUM)?
        .parse::<u32>()
        .ok()
        .and_then(|n| HorseNum::try_from(n).ok())?;
    let (finishing_position, status) = parse_finish(text(COL_FINISH)?.as_str());
    let (horse_weight, weight_change) = match text(COL_HORSE_WEIGHT) {
        Some(t) => parse_weight(&t),
        None => (None, None),
    };

    Some(HorsePastRun {
        netkeiba_race_id: race_id,
        date,
        venue,
        round,
        day,
        race_num,
        surface,
        distance,
        track_condition: text(COL_TRACK_COND)
            .and_then(|t| TrackCondition::try_from(t.as_str()).ok()),
        finishing_position,
        status,
        gate_num,
        horse_num,
        horse_name: horse_name.clone(),
        jockey: text(COL_JOCKEY).and_then(|t| JockeyName::try_from(t).ok()),
        time_seconds: text(COL_TIME).and_then(|t| TimeSeconds::try_from_mss_str(&t).ok()),
        margin: text(COL_MARGIN),
        odds: text(COL_ODDS).and_then(|t| t.parse::<f64>().ok()),
        horse_weight,
        weight_change,
        weight_carried: text(COL_WEIGHT_CARRIED).and_then(|t| t.parse::<f64>().ok()),
        popularity: text(COL_POPULARITY).and_then(|t| t.parse::<u32>().ok()),
    })
}

/// `.../race/202401020411/` から 12 桁の race_id を取り出す。
fn extract_race_id(href: &str) -> Option<String> {
    let rest = href.split("/race/").nth(1)?;
    let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    (id.len() == 12).then_some(id)
}

/// 距離セル "芝2000" / "ダ1200" / "障3000" を (馬場, 距離m) に分解。障害は `None`。
fn parse_surface_distance(raw: &str) -> Option<(Surface, u32)> {
    let surface = match raw.chars().next()? {
        '芝' => Surface::Turf,
        'ダ' => Surface::Dirt,
        _ => return None, // 障(障害) は Surface 非対応
    };
    let distance = raw
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()?;
    Some((surface, distance))
}

/// 着順セルを (着順, status) に変換。数字なら完走、"中/取/除/失" は異常終了として着順なし。
fn parse_finish(raw: &str) -> (Option<FinishingPosition>, ResultStatus) {
    let digits: String = raw.chars().take_while(char::is_ascii_digit).collect();
    if let Some(pos) = digits
        .parse::<u32>()
        .ok()
        .and_then(|n| FinishingPosition::try_from(n).ok())
    {
        return (Some(pos), ResultStatus::Finished);
    }
    let status = if raw.contains('取') {
        ResultStatus::Cancelled
    } else if raw.contains('除') {
        ResultStatus::Scratched
    } else {
        // 中(競走中止) / 失(失格) / その他不明 は「完走でない」扱い
        ResultStatus::DidNotFinish
    };
    (None, status)
}

/// 馬体重セル "464(0)" / "486(+4)" を (体重, 増減) に分解。"計不" 等は `(None, None)`。
fn parse_weight(raw: &str) -> (Option<u32>, Option<i32>) {
    let Some((w, rest)) = raw.split_once('(') else {
        return (None, None);
    };
    let weight = w.trim().parse::<u32>().ok();
    let change = rest.trim_end_matches(')').trim().parse::<i32>().ok();
    (weight, change)
}

fn sel(s: &str) -> Result<Selector> {
    Selector::parse(s).map_err(|e| Error::Parse(format!("invalid selector {s}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_numeric_is_finished_with_position() {
        let (pos, status) = parse_finish("5");
        assert_eq!(pos.map(|p| p.value()), Some(5));
        assert_eq!(status, ResultStatus::Finished);
    }

    #[test]
    fn finish_abnormal_has_no_position_and_maps_status() {
        // 中(競走中止) / 取(出走取消) / 除(競走除外) / 失(失格)
        assert_eq!(parse_finish("中"), (None, ResultStatus::DidNotFinish));
        assert_eq!(parse_finish("取"), (None, ResultStatus::Cancelled));
        assert_eq!(parse_finish("除"), (None, ResultStatus::Scratched));
        assert_eq!(parse_finish("失"), (None, ResultStatus::DidNotFinish));
    }

    #[test]
    fn surface_distance_skips_steeplechase() {
        assert_eq!(
            parse_surface_distance("芝2000"),
            Some((Surface::Turf, 2000))
        );
        assert_eq!(
            parse_surface_distance("ダ1200"),
            Some((Surface::Dirt, 1200))
        );
        assert_eq!(parse_surface_distance("障3000"), None);
    }

    #[test]
    fn weight_parses_value_and_signed_change() {
        assert_eq!(parse_weight("486(+4)"), (Some(486), Some(4)));
        assert_eq!(parse_weight("464(0)"), (Some(464), Some(0)));
        assert_eq!(parse_weight("計不"), (None, None));
    }
}
