use paddock_domain::{HorseId, HorseName, HorseNum};
use paddock_use_case::RunnerRef;
use scraper::{Html, Selector};

use super::cell_text;
use crate::error::{Error, Result};

/// 出馬表 (`race/shutuba.html`) のHTMLから出走各馬の参照情報を馬番順に抽出する。
///
/// テーブル本体（馬名等）はJS描画だが、各行の馬番セル `td.Umaban` と
/// `/horse/<id>` リンク（`title` に馬名）は静的HTMLに含まれるためここから取る。
pub fn parse_shutuba(html: &str) -> Result<Vec<RunnerRef>> {
    let doc = Html::parse_document(html);
    let row_sel = sel("table.Shutuba_Table tr.HorseList")?;
    // 馬番セルの class は枠色番号付きで "Umaban1".."Umaban8"。前方一致で拾う。
    let umaban_sel = sel("td[class^=\"Umaban\"]")?;
    let horse_sel = sel("a[href*=\"/horse/\"]")?;

    let mut runners = Vec::new();
    for row in doc.select(&row_sel) {
        let Some(horse_num) = row
            .select(&umaban_sel)
            .next()
            .and_then(|c| cell_text(&c.text().collect::<String>()))
            .and_then(|t| t.parse::<u32>().ok())
            .and_then(|n| HorseNum::try_from(n).ok())
        else {
            continue;
        };

        let Some(link) = row.select(&horse_sel).next() else {
            continue;
        };
        let Some(horse_id) = link
            .value()
            .attr("href")
            .and_then(extract_horse_id)
            .and_then(|id| HorseId::try_from(id).ok())
        else {
            continue;
        };
        let Some(horse_name) = link
            .value()
            .attr("title")
            .and_then(cell_text)
            .and_then(|n| HorseName::try_from(n).ok())
        else {
            continue;
        };

        runners.push(RunnerRef {
            horse_num,
            horse_name,
            horse_id,
        });
    }

    if runners.is_empty() {
        return Err(Error::Parse(
            "no runners found in shutuba table".to_string(),
        ));
    }
    Ok(runners)
}

/// `.../horse/2020102078"` 形式のリンクから馬IDの数字列を取り出す。
fn extract_horse_id(href: &str) -> Option<String> {
    let rest = href.split("/horse/").nth(1)?;
    let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if id.is_empty() { None } else { Some(id) }
}

fn sel(s: &str) -> Result<Selector> {
    Selector::parse(s).map_err(|e| Error::Parse(format!("invalid selector {s}: {e}")))
}
