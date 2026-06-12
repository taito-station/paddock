//! 成績 PDF の騎手列・調教師列を `mutool draw -F stext.json`（x/y 座標 + font サイズ付き）から
//! 抽出する。騎手は `parse_jockeys`、調教師は `parse_trainers`（x 帯・size 帯のみ差し替え）。
//!
//! プレーンテキスト（`-F text`）は列の x 座標と font サイズを失い、騎手名の 2 文字目と隣の
//! 馬主名が区切り無しで 1 行に連結してしまう（例 `裕信本山`）。stext.json には座標とサイズが
//! 残っているため、**馬番グリフからの x オフセット帯（騎手列）で切り出す**ことで騎手だけを
//! クリーンに取り出せる。
//!
//! 騎手/馬主の境界は **x 位置**で決める。左レース列の馬主は size 5 だが**右レース列の馬主は
//! size 6** で font サイズでは分離できないため、x オフセット帯（`JOCKEY_OFFSET_HI`）が唯一の
//! 確実な境界。
//!
//! 成績 PDF のレイアウト（左レース列の絶対 x。右レース列は約 +411）:
//! 枠番 ~20 / 馬番 ~27 / 馬名 ~49 / 斤量 ~133 / 減量印 ~149 / **騎手 ~156,177(size 6-7)** /
//! 馬主 ~193(左 size5 / 右 size6) / 調教師 ~236 / 牧場 ~276。レース番号は見出しの size≈14 グリフ。

use std::collections::HashMap;

use serde::Deserialize;

/// `race_num -> (horse_num -> 騎手名)`。騎手名は素の抽出文字列（width 正規化は
/// `JockeyName::try_from` 側が行う）。
pub type JockeyIndex = HashMap<u32, HashMap<u32, String>>;

/// `race_num -> (horse_num -> 調教師名)`。素の抽出文字列。`TrainerName` は `JockeyName` と
/// 異なり width 正規化フックを持たない（`define_string!(TrainerName, max=30)`）ため、抽出した
/// 文字列がそのまま検証・保存される。結果 PDF の調教師はフルネームの漢字表記で width 揺れは無い。
pub type TrainerIndex = HashMap<u32, HashMap<u32, String>>;

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
/// 調教師列の x 帯（馬番グリフ x からの相対オフセット）と font サイズ帯。
/// 実測（結果 PDF）では調教師は姓 offset ~207-209・名 offset ~223-228 で、馬主(~166-185)より右・
/// 牧場(~236+)より左に位置する。font サイズは**左列 4 / 右列 5** と列で異なるため帯で受ける
/// （jockey・馬主の size 6 は size 上限で除外）。x 帯が主たる分離、size 帯は二次ガード。
///
/// 既知の制約: HI(230) と牧場列(~236) の間は約 6 単位しか空かない（jockey/馬主境界 HI=161 と
/// 同様に x 帯が唯一の境界）。牧場名は仮名混じり地名（新ひだか/様似 等）で size でも除外できない
/// ため、開催場・年度差で x が数単位ずれると牧場 1 トークン目が混入しうる。レイアウト退行時は
/// `parse_trainers` の 0 件ログと統合テスト（既知調教師の完全一致）で気づく前提。
const TRAINER_OFFSET_LO: f64 = 195.0;
const TRAINER_OFFSET_HI: f64 = 230.0;
const TRAINER_SIZE: std::ops::RangeInclusive<f64> = 3.0..=5.5;
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

/// stext.json から列インデックス（`race_num -> (horse_num -> 名前)`）を構築する共通骨格。
/// 騎手・調教師の差分（列抽出関数 `extract`・ログ文言）だけを引数で受け取り、走査骨格
/// （パース → `flatten` → `race_numbers` → `horse_rows` ループ → 0 件ログ）を共有する。
///
/// - `parse_err_msg`: JSON パース失敗時の debug ログ本文（空文字列入力時は静かに無視）。
/// - `empty_msg`: トークンはあるのに 0 件だった場合（レイアウト退行の疑い）の debug ログ本文。
/// - `extract`: 馬番アンカー行（page, row_y, hn_x）から 1 件の名前を取り出す列抽出関数。
fn parse_column(
    stext_json: &str,
    parse_err_msg: &str,
    empty_msg: &str,
    extract: impl Fn(&[Tok], usize, f64, f64) -> Option<String>,
) -> HashMap<u32, HashMap<u32, String>> {
    let doc: StextDoc = match serde_json::from_str(stext_json) {
        Ok(d) => d,
        Err(e) => {
            // 空文字列（mutool 失敗）は best-effort なので静かに、非空なら JSON 破損として
            // ログする（レイアウト退行=0件ログ と切り分けられるように）。
            if !stext_json.is_empty() {
                tracing::debug!(error = %e, "{}", parse_err_msg);
            }
            return HashMap::new();
        }
    };
    let toks = flatten(&doc);
    let col_race = race_numbers(&toks);

    let mut index: HashMap<u32, HashMap<u32, String>> = HashMap::new();
    for (page, side, row_y, horse_num, hn_x) in horse_rows(&toks) {
        let Some(&race_num) = col_race.get(&(page, side)) else {
            continue;
        };
        if let Some(name) = extract(&toks, page, row_y, hn_x) {
            index
                .entry(race_num)
                .or_default()
                .entry(horse_num)
                .or_insert(name);
        }
    }
    // レイアウト定数は実測ハードコードのため、開催場・年度差でレイアウトが変わると
    // 0 件に退行しうる。トークンはあるのに 1 件も取れなかった場合は気づけるようログする
    // （取り込み自体は既存ヒューリスティックのフォールバックで継続する）。
    let total: usize = index.values().map(|m| m.len()).sum();
    if total == 0 && !toks.is_empty() {
        tracing::debug!(tokens = toks.len(), "{}", empty_msg);
    }
    index
}

/// stext.json から騎手インデックスを構築する。パース失敗時は空（呼び出し側で
/// 既存ヒューリスティックにフォールバック）。
pub fn parse_jockeys(stext_json: &str) -> JockeyIndex {
    parse_column(
        stext_json,
        "stext.json のパースに失敗。騎手はフォールバック抽出",
        "stext からの騎手抽出が 0 件。座標レイアウトが想定と異なる可能性",
        |toks, page, row_y, hn_x| {
            column_for(
                toks,
                page,
                row_y,
                hn_x,
                JOCKEY_OFFSET_LO,
                JOCKEY_OFFSET_HI,
                |size| size >= NAME_SIZE_MIN,
            )
        },
    )
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

/// 馬番アンカーと同じ行（y 近傍）の列トークンを連結して名前を作る、騎手・調教師共通の抽出。
/// 差分は **馬番グリフ x からの相対オフセット帯**（`offset_lo`/`offset_hi`）と
/// **font サイズ条件**（`size_ok`）の 2 点のみ。帯でフィルタ → x 昇順ソート → `name_token`
/// 連結 → トリム、という骨格は両者で共通。
///
/// - 騎手: `JOCKEY_OFFSET_LO/HI` + `size >= NAME_SIZE_MIN`（サイズ上限なし）。
/// - 調教師: `TRAINER_OFFSET_LO/HI` + `TRAINER_SIZE`（jockey・馬主の size 6 を上限で除外）。
fn column_for(
    toks: &[Tok],
    page: usize,
    row_y: f64,
    hn_x: f64,
    offset_lo: f64,
    offset_hi: f64,
    size_ok: impl Fn(f64) -> bool,
) -> Option<String> {
    let lo = hn_x + offset_lo;
    let hi = hn_x + offset_hi;

    let mut parts: Vec<(f64, &str)> = toks
        .iter()
        .filter(|t| {
            t.page == page
                && size_ok(t.size)
                && (t.y - row_y).abs() <= ROW_Y_TOL
                && lo <= t.x
                && t.x <= hi
        })
        .map(|t| (t.x, t.text.as_str()))
        .collect();
    parts.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut name = String::new();
    for (_, text) in parts {
        let part = name_token(text);
        if part.is_empty() {
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

/// stext.json から調教師インデックスを構築する。`parse_jockeys` と同骨格（`parse_column`）で、
/// 列抽出の x 帯と size 帯のみ調教師用に差し替える。パース失敗時は空（呼び出し側で trainer なし扱い）。
pub fn parse_trainers(stext_json: &str) -> TrainerIndex {
    parse_column(
        stext_json,
        "stext.json のパースに失敗。調教師は未取得",
        "stext からの調教師抽出が 0 件。座標レイアウトが想定と異なる可能性",
        |toks, page, row_y, hn_x| {
            column_for(
                toks,
                page,
                row_y,
                hn_x,
                TRAINER_OFFSET_LO,
                TRAINER_OFFSET_HI,
                |size| TRAINER_SIZE.contains(&size),
            )
        },
    )
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

/// 騎手・調教師名トークンの正規化（jockey/trainer 共通）。`clean_part` で減量印・馬主マーカーを
/// 処理したうえで、名前に含まれない ASCII 文字（ラテン略号 `RC`・記号）と数字を文字単位で落とす。
/// フィルタは「ASCII を全除去（半角数字もここで落ちる）＋全角数字を追加除去」で、残るのは非 ASCII
/// かつ非数字、すなわち漢字・仮名のみ。`RC武藤` のような混在トークンでもラテン部分だけ除去できる。
fn name_token(text: &str) -> String {
    clean_part(text)
        .chars()
        .filter(|c| !c.is_ascii() && !is_digit_char(*c))
        .collect()
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

    #[test]
    fn jockey_strips_latin_from_mixed_token() {
        // name_token 集約により、騎手側でも混在トークン（ラテン略号＋漢字）からラテン部を
        // 落として漢字名だけを残す（trainer 側と対称の挙動を固定する）。
        let json = doc_json(&[
            (216.0, 116.0, 14.0, "1"),
            (27.0, 191.0, 6.0, "6"),
            (156.0, 191.0, 6.0, "RC田辺"), // 混在トークン → RC 除去
            (177.0, 191.0, 6.0, "裕信"),
        ]);
        let idx = parse_jockeys(&json);
        assert_eq!(
            idx.get(&1).and_then(|m| m.get(&6)).map(String::as_str),
            Some("田辺裕信")
        );
    }

    #[test]
    fn extracts_trainer_and_excludes_jockey_owner_and_farm() {
        // 左列実測レイアウト: 馬番6 / 騎手(size6,x156-177) / 馬主(size5,x193) /
        // 調教師(size4,x236姓+x250名) / 牧場(size4,x263)。調教師だけを姓名連結で取る。
        let json = doc_json(&[
            (216.0, 116.0, 14.0, "1"),
            (27.0, 191.0, 6.0, "6"),
            (156.0, 191.0, 6.0, "田辺"),   // 騎手: size6 で trainer 帯外
            (177.0, 191.0, 6.0, "裕信"),   // 騎手
            (193.0, 191.0, 5.0, "本山"),   // 馬主: offset ~166 で TRAINER_OFFSET_LO 未満
            (236.0, 191.0, 4.0, "千葉"),   // 調教師 姓 (offset 209)
            (250.0, 191.0, 4.0, "直人"),   // 調教師 名 (offset 223)
            (263.0, 191.0, 4.0, "新ひだか"), // 牧場: offset 236 で TRAINER_OFFSET_HI 超
        ]);
        let idx = parse_trainers(&json);
        assert_eq!(
            idx.get(&1).and_then(|m| m.get(&6)).map(String::as_str),
            Some("千葉直人")
        );
    }

    #[test]
    fn right_column_trainer_uses_relative_offset() {
        // 右列（hn_x≈438）でも馬番からの相対オフセットで調教師を取る。右列は size5。
        let json = doc_json(&[
            (627.0, 67.0, 14.0, "2"),
            (438.0, 142.0, 6.0, "11"),
            (567.0, 142.0, 6.0, "横山"),     // 騎手
            (588.0, 142.0, 6.0, "和生"),     // 騎手
            (604.0, 142.0, 6.0, "秋元"),     // 馬主(size6) → 帯外
            (647.0, 142.0, 5.0, "中川"),     // 調教師 姓 (offset 209)
            (661.0, 142.0, 5.0, "公成"),     // 調教師 名 (offset 223)
            (675.0, 142.0, 4.0, "様似"),     // 牧場 (offset 237) → 帯外
        ]);
        let idx = parse_trainers(&json);
        assert_eq!(
            idx.get(&2).and_then(|m| m.get(&11)).map(String::as_str),
            Some("中川公成")
        );
    }

    #[test]
    fn trainer_excludes_record_marker_latin_token() {
        // レコード標示 `RC`（実 PDF で調教師帯に紛れた）が調教師名に混入しないこと。
        let json = doc_json(&[
            (216.0, 116.0, 14.0, "1"),
            (27.0, 169.0, 6.0, "8"),
            (156.0, 169.0, 6.0, "武藤"),  // 騎手
            (184.0, 169.0, 6.0, "雅"),    // 騎手
            (224.0, 169.0, 5.0, "RC"),    // レコード標示 (offset 197) → 除外
            (233.0, 169.0, 4.0, "武藤"),  // 調教師 姓
            (250.0, 169.0, 4.0, "善則"),  // 調教師 名
        ]);
        let idx = parse_trainers(&json);
        assert_eq!(
            idx.get(&1).and_then(|m| m.get(&8)).map(String::as_str),
            Some("武藤善則")
        );
    }

    #[test]
    fn trainer_strips_latin_from_mixed_token() {
        // レコード標示 `RC` が調教師姓と同一トークンに連結された場合でも、ASCII 英数字を
        // 文字単位で落として漢字名だけを残す（別トークンに分かれない PDF への保険）。
        let json = doc_json(&[
            (216.0, 116.0, 14.0, "1"),
            (27.0, 169.0, 6.0, "8"),
            (156.0, 169.0, 6.0, "武藤"), // 騎手
            (233.0, 169.0, 4.0, "RC武藤"), // 調教師 姓に RC が連結 (offset 206)
            (250.0, 169.0, 4.0, "善則"),   // 調教師 名
        ]);
        let idx = parse_trainers(&json);
        assert_eq!(
            idx.get(&1).and_then(|m| m.get(&8)).map(String::as_str),
            Some("武藤善則")
        );
    }

    #[test]
    fn trainer_excludes_fullwidth_weight_digits() {
        // 斤量等の全角数字（５５）が調教師帯に紛れても名前にしない（jockey と同じ数字ガード）。
        let json = doc_json(&[
            (216.0, 116.0, 14.0, "1"),
            (27.0, 169.0, 6.0, "8"),
            (156.0, 169.0, 6.0, "武藤"),
            (230.0, 169.0, 4.0, "５５"), // 全角数字 (offset 203) → 除外
            (236.0, 169.0, 4.0, "中川"), // 調教師 姓
            (250.0, 169.0, 4.0, "公成"), // 調教師 名
        ]);
        let idx = parse_trainers(&json);
        assert_eq!(
            idx.get(&1).and_then(|m| m.get(&8)).map(String::as_str),
            Some("中川公成")
        );
    }

    #[test]
    fn trainer_single_token_full_name() {
        // フルネームが 1 トークンで来るケース（実測 `加藤士津八` 等）。
        let json = doc_json(&[
            (627.0, 67.0, 14.0, "2"),
            (438.0, 131.0, 6.0, "13"),
            (567.0, 131.0, 6.0, "戸崎"),
            (588.0, 131.0, 6.0, "圭太"),
            (645.0, 131.0, 5.0, "加藤士津八"), // 調教師 (offset 207, 単一トークン)
            (675.0, 131.0, 4.0, "新ひだか"),
        ]);
        let idx = parse_trainers(&json);
        assert_eq!(
            idx.get(&2).and_then(|m| m.get(&13)).map(String::as_str),
            Some("加藤士津八")
        );
    }
}
