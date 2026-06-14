use std::sync::LazyLock;

use paddock_domain::{RaceId, RacePayouts};
use scraper::{ElementRef, Html, Selector};

use crate::error::Result;

// 払戻ブロックのセレクタ（行ごとの再コンパイルを避け static 化。`result.rs` と同方針）。
macro_rules! selector {
    ($name:ident, $css:literal) => {
        static $name: LazyLock<Selector> =
            LazyLock::new(|| Selector::parse($css).expect("static selector must be valid"));
    };
}
// 結果ページには「払戻し」「ワイド」など複数の Payout_Detail_Table が並ぶため全て拾う。
selector!(PAYOUT_TABLE, "table.Payout_Detail_Table");
selector!(ROW, "tr");
selector!(RESULT_CELL, "td.Result");
selector!(PAYOUT_CELL, "td.Payout");
selector!(RESULT_DIV_SPAN, "div span"); // 単勝/複勝の馬番（数字のある span のみ）
selector!(RESULT_UL, "ul"); // 組合せ券種の組（ul 1 つ = 1 組合せ）
selector!(UL_SPAN, "li span"); // 組合せ構成馬番

/// 券種クラス（`tr` の class 属性）→ paddock の type_label 対応。
/// `Wakuren`（枠連）は predict 非対象のため返さない（呼び出し側でスキップ）。
fn type_label_of(class: &str) -> Option<&'static str> {
    Some(match class {
        "Tansho" => "win",
        "Fukusho" => "place",
        "Umaren" => "quinella",
        "Wide" => "wide",
        "Umatan" => "exacta",
        "Fuku3" => "trio",
        "Tan3" => "trifecta",
        _ => return None,
    })
}

/// 無順（quinella/wide/trio）か。`true` なら combination_code を昇順ソートして `-` 連結する。
fn is_unordered(label: &str) -> bool {
    matches!(label, "quinella" | "wide" | "trio")
}

/// netkeiba レース結果ページ (`race/result.html`) の払戻ブロック
/// (`table.Payout_Detail_Table`) から確定払戻を抽出して [`RacePayouts`] に詰める。
///
/// 組合せコードは [`paddock_domain::BetCombination::combination_code`] と一致する形式へ
/// 正規化する（無順は馬番昇順 `-` 連結 / 順序付きは出現順 `>` 連結）。払戻ブロックが
/// 1 件も無い・的中行が無いレースは空の [`RacePayouts`]（＝未確定）を返す。
pub fn parse_race_payouts(html: &str, race_id: RaceId) -> Result<RacePayouts> {
    let doc = Html::parse_document(html);
    let mut payouts = RacePayouts::empty(race_id);

    for table in doc.select(&PAYOUT_TABLE) {
        for row in table.select(&ROW) {
            extract_row(&row, &mut payouts);
        }
    }
    Ok(payouts)
}

/// 1 行（`<tr class="...">`）から券種と払戻を抽出して `payouts` に追加する。
/// 対象外券種（枠連等）・的中行を欠く行は無視する。
fn extract_row(row: &ElementRef, payouts: &mut RacePayouts) {
    let Some(label) = row.value().attr("class").and_then(type_label_of) else {
        return;
    };
    let Some(result_cell) = row.select(&RESULT_CELL).next() else {
        return;
    };
    let Some(payout_cell) = row.select(&PAYOUT_CELL).next() else {
        return;
    };
    let amounts = parse_payouts(&payout_cell);

    let combos = if label == "win" || label == "place" {
        // 単勝/複勝: div>span の馬番（空 span を除く）。複勝は馬番ごとに 1 点。
        parse_single_nums(&result_cell)
    } else {
        // 組合せ券種: ul ごとに 1 組合せ。出現順（着順）で並ぶ。
        parse_combos(&result_cell, is_unordered(label))
    };

    // 組合せ[i] ↔ 配当[i] を位置で対応付ける。件数が食い違う行は対応がズレて誤った
    // 馬番に配当を貼るおそれがあるため、当該券種ごと skip して warn を出す（払戻金額に
    // 直結するため沈黙させない）。複勝・ワイドのように 1 行に複数組×複数配当が並ぶ券種で
    // HTML 構造が想定とズレた場合の保険。
    if combos.len() != amounts.len() {
        tracing::warn!(
            bet_type = label,
            combos = combos.len(),
            amounts = amounts.len(),
            "払戻の組合せ数と配当数が不一致のため当該券種をスキップ"
        );
        return;
    }
    for (combo, payoff) in combos.iter().zip(amounts.iter()) {
        payouts.insert(label, combo.clone(), *payoff);
    }
}

/// `td.Result` 内の `div span` から数字のある馬番文字列を出現順に取り出す（単勝/複勝）。
fn parse_single_nums(result_cell: &ElementRef) -> Vec<String> {
    result_cell
        .select(&RESULT_DIV_SPAN)
        .filter_map(|s| {
            let t = s.text().collect::<String>();
            let t = t.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        })
        .collect()
}

/// `td.Result` 内の各 `ul`（= 1 組合せ）から構成馬番を取り出し、combination_code へ正規化する。
/// `unordered` なら昇順ソートで `-` 連結、順序付きなら出現順（着順）で `>` 連結。
fn parse_combos(result_cell: &ElementRef, unordered: bool) -> Vec<String> {
    result_cell
        .select(&RESULT_UL)
        .filter_map(|ul| {
            let nums: Vec<u32> = ul
                .select(&UL_SPAN)
                .filter_map(|s| s.text().collect::<String>().trim().parse::<u32>().ok())
                .collect();
            if nums.is_empty() {
                return None;
            }
            Some(if unordered {
                let mut sorted = nums.clone();
                sorted.sort_unstable();
                sorted
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join("-")
            } else {
                nums.iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(">")
            })
        })
        .collect()
}

/// `td.Payout` の配当文字列（例 `280円<br />110円` / `1,010円`）を u32 円のリストにする。
///
/// `<br>` や改行の有無に依存せず、セル全体のテキストを走査して「数字（桁区切りカンマを含む）
/// に直後 `円` が続く」並びを 1 配当ずつ取り出す。これにより `280円110円` のように区切りが
/// 無く連結したケースでも `[280, 110]` と正しく分割でき、`5人気` のような円で終わらない数字は
/// 拾わない（人気欄混入への保険）。
fn parse_payouts(payout_cell: &ElementRef) -> Vec<u32> {
    let text: String = payout_cell.text().collect();
    let mut out = Vec::new();
    let mut digits = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else if ch == ',' {
            // 桁区切りカンマは無視して数字の連結を継続する。
        } else {
            // 数字列の直後が `円` のときだけ 1 配当として確定する。
            if ch == '円'
                && !digits.is_empty()
                && let Ok(n) = digits.parse::<u32>()
            {
                out.push(n);
            }
            digits.clear();
        }
    }
    out
}
