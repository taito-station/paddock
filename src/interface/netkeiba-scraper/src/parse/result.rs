use paddock_domain::{HorseNum, JockeyName, TimeSeconds, TrainerName};
use paddock_use_case::netkeiba_scraper::ResultRow;
use scraper::{ElementRef, Html, Selector};

use super::cell_text;
use super::horse_history::{parse_finish, parse_weight};
use crate::error::{Error, Result};

/// netkeiba レース結果ページ (`race/result.html?race_id=...`) の確定成績テーブルから
/// 全出走馬の成績行を抽出する。
///
/// 列はクラスで識別する（`card.rs` と同方針）。jockey は `td.Jockey` のセルテキスト、trainer は
/// `td.Trainer a` の `title` 属性で、いずれも netkeiba の**略名**（出馬表 entry と同一表記）。
/// 着順 `td.Result_Num`（`中/取/除/失` は着順なし＋status）、馬番 `td.Num.Txt_C` を更新キーに使う。
/// race メタ（場/距離/馬場）は本経路では扱わない（既存 races 行は触らず results のみ更新するため）。
pub fn parse_race_result(html: &str, netkeiba_race_id: &str) -> Result<Vec<ResultRow>> {
    let doc = Html::parse_document(html);
    let row_sel = sel("table#All_Result_Table tr")?;

    let rows: Vec<ResultRow> = doc.select(&row_sel).filter_map(extract_row).collect();

    if rows.is_empty() {
        return Err(Error::Parse(format!(
            "結果テーブルから着順を抽出できませんでした: race_id={netkeiba_race_id}"
        )));
    }
    Ok(rows)
}

/// 1 行（`<tr>`）から成績を抽出する。着順セルか馬番セルを欠く行（ヘッダ等）は `None`。
fn extract_row(row: ElementRef) -> Option<ResultRow> {
    // 着順セル（`中/取/除/失` も含む）。無ければデータ行でない。
    let finish_raw = text_of(&row, "td.Result_Num")?;
    let (finishing_position, status) = parse_finish(&finish_raw);

    let horse_num = text_of(&row, "td.Num.Txt_C")?
        .parse::<u32>()
        .ok()
        .and_then(|n| HorseNum::try_from(n).ok())?;

    // 騎手は `td.Jockey` のセルテキスト（略名、title 属性は無い）。
    let jockey = text_of(&row, "td.Jockey").and_then(|t| JockeyName::try_from(t).ok());
    // 調教師は `td.Trainer a` の title 属性（略名。"栗東 宮地" の表示でなく title="宮地"）。
    let trainer = attr_title(&row, "td.Trainer a").and_then(|t| TrainerName::try_from(t).ok());

    let time_seconds = text_of(&row, "td.Time").and_then(|t| TimeSeconds::try_from_mss_str(&t).ok());
    let weight_carried = text_of(&row, "td.Jockey_Info").and_then(|t| t.parse::<f64>().ok());
    let popularity = text_of(&row, "td.Odds.Txt_C").and_then(|t| t.parse::<u32>().ok());
    let odds = text_of(&row, "td.Odds.Txt_R").and_then(|t| t.parse::<f64>().ok());
    let (horse_weight, weight_change) = match text_of(&row, "td.Weight") {
        Some(t) => parse_weight(&t),
        None => (None, None),
    };

    Some(ResultRow {
        horse_num,
        finishing_position,
        status,
        jockey,
        trainer,
        time_seconds,
        odds,
        horse_weight,
        weight_change,
        weight_carried,
        popularity,
    })
}

/// 行内で `selector` に一致する最初のセルの正規化テキストを返す。
fn text_of(row: &ElementRef, selector: &str) -> Option<String> {
    let s = Selector::parse(selector).ok()?;
    let cell = row.select(&s).next()?;
    cell_text(&cell.text().collect::<String>())
}

/// 行内で `selector` に一致する最初の要素の `title` 属性（正規化済み）を返す。
fn attr_title(row: &ElementRef, selector: &str) -> Option<String> {
    let s = Selector::parse(selector).ok()?;
    let el = row.select(&s).next()?;
    el.value().attr("title").and_then(cell_text)
}

fn sel(s: &str) -> Result<Selector> {
    Selector::parse(s).map_err(|e| Error::Parse(format!("invalid selector {s}: {e}")))
}
