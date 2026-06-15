use std::collections::HashSet;
use std::sync::LazyLock;

use paddock_domain::{HorseNum, JockeyName, ResultStatus, TimeSeconds, TrainerName};
use paddock_use_case::netkeiba_scraper::ResultRow;
use scraper::{ElementRef, Html, Selector};

use super::cell_text;
use super::horse_history::{parse_finish, parse_weight};
use crate::error::{Error, Result};

// 結果テーブルの列セレクタ（行ごとの再コンパイルを避け static 化。`card.rs` と同方針）。
// セレクタは全てリテラル固定なので unwrap/expect で十分（初回アクセス時のみ評価）。
macro_rules! selector {
    ($name:ident, $css:literal) => {
        static $name: LazyLock<Selector> =
            LazyLock::new(|| Selector::parse($css).expect("static selector must be valid"));
    };
}
selector!(ROW, "table#All_Result_Table tr");
selector!(FINISH, "td.Result_Num"); // 着順（中/取/除 を含む）
selector!(HORSE_NUM, "td.Num.Txt_C"); // 馬番（枠は td.Num.WakuN で別）
selector!(JOCKEY, "td.Jockey"); // 騎手（略名、セルテキスト）
selector!(TRAINER, "td.Trainer a"); // 調教師リンク（title=略名）
selector!(WEIGHT_CARRIED, "td.Jockey_Info"); // 斤量
selector!(TIME, "td.Time"); // 先頭が走破タイム
selector!(POPULARITY, "td.Odds.Txt_C"); // 人気
selector!(ODDS, "td.Odds.Txt_R"); // 単勝オッズ
selector!(WEIGHT, "td.Weight"); // 馬体重

/// netkeiba レース結果ページ (`race/result.html?race_id=...`) の確定成績テーブルから
/// 全出走馬の成績行を抽出する。
///
/// 列はクラスで識別する（`card.rs` と同方針）。jockey は `td.Jockey` のセルテキスト、trainer は
/// `td.Trainer a` の `title` 属性（無ければセルテキストへフォールバック）で、いずれも netkeiba の
/// **略名**（出馬表 entry と同一表記）。着順 `td.Result_Num`（`中/取/除/失` は着順なし＋status）、
/// 馬番 `td.Num.Txt_C` を更新キーに使う。race メタ（場/距離/馬場）は本経路では扱わない
/// （既存 races 行は触らず results のみ更新するため）。
pub fn parse_race_result(html: &str, netkeiba_race_id: &str) -> Result<Vec<ResultRow>> {
    let doc = Html::parse_document(html);
    let rows: Vec<ResultRow> = doc.select(&ROW).filter_map(extract_row).collect();

    if rows.is_empty() {
        return Err(Error::Parse(format!(
            "結果テーブルから着順を抽出できませんでした: race_id={netkeiba_race_id}"
        )));
    }
    Ok(rows)
}

/// 結果テーブルから**返還対象**（出走取消=取 / 競走除外=除）の馬番集合を抽出する。
///
/// 自動精算（#129）で組番に非出走馬を含む買い目を全額返還するために使う。`parse_race_payouts`
/// と同じ結果ページ HTML（`doc`）を使い回す前提（追加取得なし）。中止(中)・失格(失) は出走済みの
/// ため返還対象に含めない（`parse_finish` の status 分類に準拠）。該当馬が無ければ空集合を返す。
///
/// 馬番は生 `u32` で集める（`HorseNum` の範囲検証は通さない）。照合相手の `combo_nums`
/// （domain）も生 `u32` で対称なので範囲検証は不要、かつ結果表の馬番セルは常に数値のため
/// `parse` 失敗は実データでは起きない。
pub(crate) fn scratched_horse_nums(doc: &Html) -> HashSet<u32> {
    doc.select(&ROW)
        .filter_map(|row| {
            let (_, status) = parse_finish(&text_of(&row, &FINISH)?);
            if matches!(status, ResultStatus::Cancelled | ResultStatus::Scratched) {
                text_of(&row, &HORSE_NUM)?.parse::<u32>().ok()
            } else {
                None
            }
        })
        .collect()
}

/// 結果テーブルの**全出走馬が取消(取)/除外(除)**か（開催中止・全馬取消の判定、#131）。
///
/// 着順セル `td.Result_Num` を持つ出走行が **1 行以上あり、その全行**の status が
/// `Cancelled|Scratched` のとき `true`。0 行（成績表が無い空ページ・エラーページ・未確定）や、
/// 完走・中止(中)・失格(失) の行が 1 つでも混在するレースは `false` を返す。
///
/// 呼び出し側（`parse_race_payouts`）は払戻ブロックが無い（`is_empty()`）レースに限り本判定を
/// 用い、全額返還レースとして印を付ける。完走行が混在する＝払戻が存在し得るため、払戻パースが
/// 構造変化で空になった場合でも全額返還へ誤分類せず pending に倒れる（安全側）。
pub(crate) fn race_fully_refunded(doc: &Html) -> bool {
    let mut any_runner = false;
    for row in doc.select(&ROW) {
        let Some(finish) = text_of(&row, &FINISH) else {
            continue; // ヘッダ等、着順セルを欠く行は出走行でない。
        };
        any_runner = true;
        let (_, status) = parse_finish(&finish);
        if !matches!(status, ResultStatus::Cancelled | ResultStatus::Scratched) {
            return false;
        }
    }
    any_runner
}

/// 1 行（`<tr>`）から成績を抽出する。着順セルか馬番セルを欠く行（ヘッダ等）は `None`。
fn extract_row(row: ElementRef) -> Option<ResultRow> {
    // 着順セル（`中/取/除/失` も含む）。無ければデータ行でない。
    let (finishing_position, status) = parse_finish(&text_of(&row, &FINISH)?);
    let horse_num = text_of(&row, &HORSE_NUM)?
        .parse::<u32>()
        .ok()
        .and_then(|n| HorseNum::try_from(n).ok())?;

    let jockey = text_of(&row, &JOCKEY).and_then(|t| JockeyName::try_from(t).ok());
    // 調教師は `td.Trainer a` の title を優先し、無い行はセルテキスト（"栗東 宮地"→末尾語）でなく
    // リンクテキストへフォールバックする（title 欠落への保険。jockey と取得経路を揃える）。
    let trainer = trainer_name(&row).and_then(|t| TrainerName::try_from(t).ok());

    let time_seconds = text_of(&row, &TIME).and_then(|t| TimeSeconds::try_from_mss_str(&t).ok());
    let weight_carried = text_of(&row, &WEIGHT_CARRIED).and_then(|t| t.parse::<f64>().ok());
    let popularity = text_of(&row, &POPULARITY).and_then(|t| t.parse::<u32>().ok());
    let odds = text_of(&row, &ODDS).and_then(|t| t.parse::<f64>().ok());
    let (horse_weight, weight_change) = match text_of(&row, &WEIGHT) {
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

/// 調教師名（略名）を取る。`td.Trainer a` の `title` 属性を優先し、無ければリンクのテキスト
/// （`TrainerNameSpan` の表示名）にフォールバックする。
fn trainer_name(row: &ElementRef) -> Option<String> {
    let link = row.select(&TRAINER).next()?;
    link.value()
        .attr("title")
        .and_then(cell_text)
        .or_else(|| cell_text(&link.text().collect::<String>()))
}

/// 行内で `selector` に一致する最初のセルの正規化テキストを返す。
fn text_of(row: &ElementRef, selector: &Selector) -> Option<String> {
    let cell = row.select(selector).next()?;
    cell_text(&cell.text().collect::<String>())
}
