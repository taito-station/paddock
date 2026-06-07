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
/// 名前トークン（騎手・馬名）の font サイズ下限。左列の馬主は size 5 だが、**右列の馬主は
/// size 6** のためサイズだけでは騎手/馬主を分離できない（分離は x 帯 = JOCKEY_OFFSET_HI で行う）。
/// ここでは斤量列の小さい数字や地方・牧場（size 3-4）を落とす二次ガード。font サイズの端数
/// （5.98 等）でも騎手を取りこぼさないよう 5.5 をしきい値にする。
const NAME_SIZE_MIN: f64 = 5.5;
/// 馬番グリフの x 帯（左列 / 右列）。枠番(~20/431)は含めない。
const HN_X_LEFT: std::ops::RangeInclusive<f64> = 24.0..=34.0;
const HN_X_RIGHT: std::ops::RangeInclusive<f64> = 435.0..=445.0;
/// 騎手列の x 帯（馬番グリフ x からの相対オフセット）。
/// 実測（samples/2026-3nakayama6.pdf）では騎手 2 文字目が最大オフセット ~157、馬主先頭が
/// ~165-166。**右列の馬主は size 6 で size 分離が効かないため、この x 上限が騎手/馬主の唯一の
/// 確実な境界**。両者の間（~161）で切る。
const JOCKEY_OFFSET_LO: f64 = 118.0;
const JOCKEY_OFFSET_HI: f64 = 161.0;
/// 同一行とみなす y 許容。
const ROW_Y_TOL: f64 = 3.0;

/// 減量騎手印（騎手名の前に付くため除去する）。
fn is_reduction_marker(c: char) -> bool {
    matches!(c, '▲' | '△' | '☆' | '★' | '◇' | '◆')
}

/// 騎手名トークンの右端で打ち切るマーカー。`氏`（馬主氏名マーカー）と全角空白は馬主
/// セクションの開始を表す。制御文字（C0/C1）と置換文字 U+FFFD はフォント欠落グリフで、
/// 騎手 2 文字目に連結することがあるため同様に打ち切る（ここで除去しないと `JockeyName`
/// の制御文字検査で名前全体が弾かれフォールバックに落ちる）。
fn is_stop_marker(c: char) -> bool {
    c.is_control() || c == '\u{FFFD}' || c == '氏' || c == '\u{3000}'
}

/// 半角・全角いずれかの数字か。
fn is_digit_char(c: char) -> bool {
    c.is_ascii_digit() || ('０'..='９').contains(&c)
}

fn side_of(x: f64) -> u8 {
    if x < COL_SPLIT_X { 0 } else { 1 }
}

/// stext.json から騎手インデックスを構築する。パース失敗時は空（呼び出し側で
/// 既存ヒューリスティックにフォールバック）。
pub fn parse_jockeys(stext_json: &str) -> JockeyIndex {
    let doc: StextDoc = match serde_json::from_str(stext_json) {
        Ok(d) => d,
        Err(e) => {
            // 空文字列（mutool 失敗）は best-effort なので静かに、非空なら JSON 破損として
            // ログする（レイアウト退行=0件ログ と切り分けられるように）。
            if !stext_json.is_empty() {
                tracing::debug!(error = %e, "stext.json のパースに失敗。騎手はフォールバック抽出");
            }
            return JockeyIndex::new();
        }
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
    // レイアウト定数は実測ハードコードのため、開催場・年度差でレイアウトが変わると
    // 0 件に退行しうる。トークンはあるのに 1 件も取れなかった場合は気づけるようログする
    // （取り込み自体は既存ヒューリスティックのフォールバックで継続する）。
    let total: usize = index.values().map(|m| m.len()).sum();
    if total == 0 && !toks.is_empty() {
        tracing::debug!(
            tokens = toks.len(),
            "stext からの騎手抽出が 0 件。座標レイアウトが想定と異なる可能性"
        );
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
///
/// 万一同じ `(page, side)` にサイズ帯該当の数字が複数あった場合は、**最上段（最小 y）**を
/// 見出しとして採用する（本文中の大きい数字の誤検出に対する保険）。
fn race_numbers(toks: &[Tok]) -> HashMap<(usize, u8), u32> {
    // (page, side) -> (header_y, race_num)
    let mut map: HashMap<(usize, u8), (f64, u32)> = HashMap::new();
    for t in toks {
        if RACE_NUM_SIZE.contains(&t.size)
            && let Ok(n) = t.text.trim().parse::<u32>()
            && (1..=12).contains(&n)
        {
            let key = (t.page, side_of(t.x));
            match map.get(&key) {
                Some(&(y, _)) if y <= t.y => {}
                _ => {
                    map.insert(key, (t.y, n));
                }
            }
        }
    }
    map.into_iter().map(|(k, (_, n))| (k, n)).collect()
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
        // 純数字（斤量・減量数値。半角/全角とも）パートは騎手名ではないので取り込まない。
        if part.is_empty() || part.chars().all(is_digit_char) {
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
    fn excludes_right_column_size6_owner_surname() {
        // 右列の馬主サーネームは **size 6**（左列の size 5 と異なる）ため、サイズでは
        // 騎手と分離できない。馬番からのオフセット（馬主先頭 ~165）が JOCKEY_OFFSET_HI を
        // 超えることで x 帯境界として除外されること（実 PDF の `横山典弘秋元` 再発防止）。
        let json = doc_json(&[
            (627.0, 67.0, 14.0, "2"),
            (439.0, 131.0, 6.0, "6"),
            (568.0, 131.0, 6.0, "横山"), // 騎手 part1 (offset 129)
            (589.0, 131.0, 6.0, "典弘"), // 騎手 part2 (offset 150)
            (604.0, 131.0, 6.0, "秋元"), // 馬主サーネーム size6 (offset 165) → 除外
            (624.0, 131.0, 6.0, "竜弥氏"), // 馬主 (offset 185) → 除外
        ]);
        let idx = parse_jockeys(&json);
        assert_eq!(
            idx.get(&2).and_then(|m| m.get(&6)).map(String::as_str),
            Some("横山典弘")
        );
    }

    #[test]
    fn excludes_owner_for_two_digit_right_column_horse_num() {
        // 2 桁馬番は馬番グリフ x がわずかに左（実測 ~438）。馬主(~604, 馬番からのオフセット
        // 166)が騎手帯に入らないこと（馬番 x の桁数差でも境界が保たれることの確認）。
        let json = doc_json(&[
            (627.0, 67.0, 14.0, "2"),
            (438.0, 131.0, 6.0, "12"),
            (567.0, 131.0, 6.0, "三浦"),
            (588.0, 131.0, 6.0, "皇成"),
            (604.0, 131.0, 6.0, "ゴドルフィン"), // 馬主 (offset 166) → 除外
        ]);
        let idx = parse_jockeys(&json);
        assert_eq!(
            idx.get(&2).and_then(|m| m.get(&12)).map(String::as_str),
            Some("三浦皇成")
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
