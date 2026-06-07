//! 成績 PDF の騎手列を `mutool draw -F stext.json`（x/y 座標 + font サイズ付き）から抽出する。
//!
//! プレーンテキスト（`-F text`）は列の x 座標と font サイズを失い、騎手名の 2 文字目と隣の
//! 馬主名が区切り無しで 1 行に連結してしまう（例 `裕信本山`）。stext.json には座標とサイズが
//! 残っており、**行内で font サイズが 6→5 に落ちる位置が騎手/馬主の境界**になるため、騎手列
//! だけをクリーンに切り出せる。
//!
//! 成績 PDF のレイアウト（左レース列の絶対 x。右レース列は約 +411）:
//! 枠番 ~20 / 馬番 ~27 / 馬名 ~49 / 斤量 ~133 / 減量印 ~149 / **騎手 ~156,177(size≥6)** /
//! 馬主 ~193(size5) / 調教師 ~236(size4) / 牧場 ~276。レース番号は見出しの size≈14 グリフ。

use std::collections::HashMap;

use serde::Deserialize;

/// `race_num -> (horse_num -> 騎手名)`。騎手名は素の抽出文字列（width 正規化は
/// `JockeyName::try_from` 側が行う）。
pub type JockeyIndex = HashMap<u32, HashMap<u32, String>>;

#[derive(Deserialize)]
struct StextDoc {
    #[serde(default)]
    pages: Vec<StextPage>,
}

#[derive(Deserialize)]
struct StextPage {
    #[serde(default)]
    blocks: Vec<StextBlock>,
}

#[derive(Deserialize)]
struct StextBlock {
    #[serde(default)]
    lines: Vec<StextLine>,
}

#[derive(Deserialize)]
struct StextLine {
    x: f64,
    y: f64,
    font: StextFont,
    text: String,
}

#[derive(Deserialize)]
struct StextFont {
    size: f64,
}

/// 平坦化したトークン（ページ番号付き）。
struct Tok {
    page: usize,
    x: f64,
    y: f64,
    size: f64,
    text: String,
}

// ── レイアウト定数 ──────────────────────────────────────────────────────────
/// 左右レース列の境界 x（これ未満が左列、以上が右列）。
const COL_SPLIT_X: f64 = 420.0;
/// レース番号グリフの font サイズ帯（見出しの大きい数字）。
const RACE_NUM_SIZE: std::ops::RangeInclusive<f64> = 12.5..=15.5;
/// 騎手・馬名は size 6-7、馬主は size 5。この閾値で馬主以降を除外する。
const NAME_SIZE_MIN: f64 = 6.0;
/// 馬番グリフの x 帯（左列 / 右列）。枠番(~20/431)は含めない。
const HN_X_LEFT: std::ops::RangeInclusive<f64> = 24.0..=34.0;
const HN_X_RIGHT: std::ops::RangeInclusive<f64> = 435.0..=445.0;
/// 騎手列の x 帯（馬番グリフ x からの相対オフセット）。馬主(オフセット~166)の手前まで。
const JOCKEY_OFFSET_LO: f64 = 118.0;
const JOCKEY_OFFSET_HI: f64 = 165.0;
/// 同一行とみなす y 許容。
const ROW_Y_TOL: f64 = 3.0;

/// 減量騎手印（騎手名の前に付くため除去する）。
fn is_reduction_marker(c: char) -> bool {
    matches!(c, '▲' | '△' | '☆' | '★' | '◇' | '◆')
}

/// 騎手名の右端で打ち切るマーカー（馬主セクション開始・フォント欠落の置換文字など）。
fn is_stop_marker(c: char) -> bool {
    matches!(c, '\u{0080}'..='\u{009F}') || c == '\u{FFFD}' || c == '氏' || c == '\u{3000}'
}

fn side_of(x: f64) -> u8 {
    if x < COL_SPLIT_X { 0 } else { 1 }
}

/// stext.json から騎手インデックスを構築する。パース失敗時は空（呼び出し側で
/// 既存ヒューリスティックにフォールバック）。
pub fn parse_jockeys(stext_json: &str) -> JockeyIndex {
    let doc: StextDoc = match serde_json::from_str(stext_json) {
        Ok(d) => d,
        Err(_) => return JockeyIndex::new(),
    };
    let toks = flatten(&doc);
    let col_race = race_numbers(&toks);

    let mut index: JockeyIndex = HashMap::new();
    for (page, side, row_y, horse_num, hn_x) in horse_rows(&toks) {
        let Some(&race_num) = col_race.get(&(page, side)) else {
            continue;
        };
        if let Some(jockey) = jockey_for(&toks, page, row_y, hn_x) {
            index
                .entry(race_num)
                .or_default()
                .entry(horse_num)
                .or_insert(jockey);
        }
    }
    index
}

fn flatten(doc: &StextDoc) -> Vec<Tok> {
    let mut out = Vec::new();
    for (page, p) in doc.pages.iter().enumerate() {
        for block in &p.blocks {
            for line in &block.lines {
                out.push(Tok {
                    page,
                    x: line.x,
                    y: line.y,
                    size: line.font.size,
                    text: line.text.clone(),
                });
            }
        }
    }
    out
}

/// レース番号（見出しの size≈14 数字グリフ）から `(page, side) -> race_num` を作る。
fn race_numbers(toks: &[Tok]) -> HashMap<(usize, u8), u32> {
    let mut map = HashMap::new();
    for t in toks {
        if RACE_NUM_SIZE.contains(&t.size)
            && let Ok(n) = t.text.trim().parse::<u32>()
            && (1..=12).contains(&n)
        {
            map.insert((t.page, side_of(t.x)), n);
        }
    }
    map
}

/// 馬番グリフ（馬番 x 帯・size≥6・1..=18 の数字）を行アンカーとして集める。
/// 返り値: `(page, side, y, horse_num, hn_x)`。
fn horse_rows(toks: &[Tok]) -> Vec<(usize, u8, f64, u32, f64)> {
    let mut rows = Vec::new();
    for t in toks {
        if t.size < NAME_SIZE_MIN {
            continue;
        }
        let side = side_of(t.x);
        let in_band = match side {
            0 => HN_X_LEFT.contains(&t.x),
            _ => HN_X_RIGHT.contains(&t.x),
        };
        if !in_band {
            continue;
        }
        let trimmed = t.text.trim();
        if let Ok(n) = trimmed.parse::<u32>()
            && (1..=18).contains(&n)
        {
            rows.push((t.page, side, t.y, n, t.x));
        }
    }
    rows
}

/// 馬番アンカーと同じ行（y 近傍）の騎手列トークンを連結して騎手名を作る。
fn jockey_for(toks: &[Tok], page: usize, row_y: f64, hn_x: f64) -> Option<String> {
    let lo = hn_x + JOCKEY_OFFSET_LO;
    let hi = hn_x + JOCKEY_OFFSET_HI;

    let mut parts: Vec<(f64, &str)> = toks
        .iter()
        .filter(|t| {
            t.page == page
                && t.size >= NAME_SIZE_MIN
                && (t.y - row_y).abs() <= ROW_Y_TOL
                && lo <= t.x
                && t.x <= hi
        })
        .map(|t| (t.x, t.text.as_str()))
        .collect();
    parts.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut name = String::new();
    for (_, text) in parts {
        let part = clean_part(text);
        // 純数字（斤量・減量数値）パートは騎手名ではないので取り込まない。
        if part.is_empty() || part.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        name.push_str(&part);
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(name)
}

/// 1 トークンから減量印を除去し、馬主マーカーで打ち切る。
fn clean_part(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        if is_reduction_marker(c) {
            continue;
        }
        if is_stop_marker(c) {
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_json(lines: &[(f64, f64, f64, &str)]) -> String {
        // 1 ページ・1 ブロックに与えられた行を詰めた stext.json を組み立てる。
        let items: Vec<String> = lines
            .iter()
            .map(|(x, y, s, t)| {
                format!(
                    r#"{{"x":{x},"y":{y},"font":{{"size":{s}}},"text":"{t}"}}"#,
                    t = t
                )
            })
            .collect();
        format!(
            r#"{{"pages":[{{"blocks":[{{"lines":[{}]}}]}}]}}"#,
            items.join(",")
        )
    }

    #[test]
    fn extracts_jockey_and_excludes_owner_by_size() {
        // 見出し race_num=1（size14, 左列）/ 馬番 6（size6, x27）/
        // 騎手 田辺(size6,x156) 裕信(size6,x177) / 馬主 本山(size5,x193)
        let json = doc_json(&[
            (216.0, 116.0, 14.0, "1"),
            (27.0, 191.0, 6.0, "6"),
            (156.0, 191.0, 6.0, "田辺"),
            (177.0, 191.0, 6.0, "裕信"),
            (193.0, 191.0, 5.0, "本山"), // 馬主: size5 で除外される
            (236.0, 191.0, 4.0, "松永"), // 調教師: size4 で除外される
        ]);
        let idx = parse_jockeys(&json);
        assert_eq!(
            idx.get(&1).and_then(|m| m.get(&6)).map(String::as_str),
            Some("田辺裕信")
        );
    }

    #[test]
    fn strips_reduction_marker_and_replacement_char() {
        // 減量印 ▲ 付き、置換文字 � が騎手 2 文字目に連結したケース
        let json = doc_json(&[
            (216.0, 116.0, 14.0, "1"),
            (27.0, 191.0, 6.0, "8"),
            (149.0, 191.0, 6.0, "▲田山"),
            (177.0, 191.0, 6.0, "旺佑\u{FFFD}"),
        ]);
        let idx = parse_jockeys(&json);
        assert_eq!(
            idx.get(&1).and_then(|m| m.get(&8)).map(String::as_str),
            Some("田山旺佑")
        );
    }

    #[test]
    fn right_column_maps_to_its_race_number() {
        // 右列（x>=420）の見出し 2、馬番 3、騎手 横山和生
        let json = doc_json(&[
            (627.0, 67.0, 14.0, "2"),
            (438.0, 191.0, 6.0, "3"),
            (567.0, 191.0, 6.0, "横山"),
            (588.0, 191.0, 6.0, "和生"),
            (604.0, 191.0, 5.0, "ロードホースクラブ"),
        ]);
        let idx = parse_jockeys(&json);
        assert_eq!(
            idx.get(&2).and_then(|m| m.get(&3)).map(String::as_str),
            Some("横山和生")
        );
    }

    #[test]
    fn ignores_weight_digits_in_jockey_band() {
        // 騎手帯に純数字（減量斤量など）が紛れても騎手名にしない
        let json = doc_json(&[
            (216.0, 116.0, 14.0, "1"),
            (27.0, 191.0, 6.0, "5"),
            (150.0, 191.0, 6.0, "52"), // 純数字 → 無視
            (160.0, 191.0, 6.0, "岩田"),
            (181.0, 191.0, 6.0, "望来"),
        ]);
        let idx = parse_jockeys(&json);
        assert_eq!(
            idx.get(&1).and_then(|m| m.get(&5)).map(String::as_str),
            Some("岩田望来")
        );
    }
}
