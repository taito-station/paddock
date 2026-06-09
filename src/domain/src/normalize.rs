//! 馬名・騎手名の正規化。取り込み（成績/出馬表パース）と検索（analyze）で同一表現に揃え、
//! 表記ゆれによる取りこぼしを防ぐ。値オブジェクト（`HorseName`/`JockeyName`）の `normalize`
//! フックから呼ばれ、`TryFrom` の length/control 検証より前に適用される。

/// 名前を正規化する:
/// - 全角英字/数字 → 半角、全角ピリオド `．`(U+FF0E) → `.`
/// - 半角カタカナ → 全角カタカナ（濁点 `ﾞ`・半濁点 `ﾟ` は直前の仮名へ合成）
/// - 前後の空白を除去
///
/// 冪等（正規化済み文字列を再投入しても変化しない）。
pub(crate) fn normalize_name(value: String) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];

        // 全角 ASCII（英字/数字）・全角ピリオド → 半角。
        let ascii = match c {
            'Ａ'..='Ｚ' => Some((b'A' + (c as u32 - 'Ａ' as u32) as u8) as char),
            'ａ'..='ｚ' => Some((b'a' + (c as u32 - 'ａ' as u32) as u8) as char),
            '０'..='９' => Some((b'0' + (c as u32 - '０' as u32) as u8) as char),
            '．' => Some('.'),
            _ => None,
        };
        if let Some(m) = ascii {
            out.push(m);
            i += 1;
            continue;
        }

        // 半角カタカナ → 全角。後続が濁点/半濁点なら合成して 1 文字にする。
        if let Some(base) = halfwidth_kana_to_fullwidth(c) {
            match chars.get(i + 1).copied() {
                Some('ﾞ') => {
                    if let Some(v) = compose_voiced(base) {
                        out.push(v);
                        i += 2;
                        continue;
                    }
                }
                Some('ﾟ') => {
                    if let Some(v) = compose_semivoiced(base) {
                        out.push(v);
                        i += 2;
                        continue;
                    }
                }
                _ => {}
            }
            out.push(base);
            i += 1;
            continue;
        }

        out.push(c);
        i += 1;
    }
    out.trim().to_string()
}

/// 半角カタカナ 1 文字を全角カタカナ（濁点なしの基底）へ。対象外は `None`。
fn halfwidth_kana_to_fullwidth(c: char) -> Option<char> {
    let mapped = match c {
        '･' => '・',
        'ｰ' => 'ー',
        'ｦ' => 'ヲ',
        'ｧ' => 'ァ',
        'ｨ' => 'ィ',
        'ｩ' => 'ゥ',
        'ｪ' => 'ェ',
        'ｫ' => 'ォ',
        'ｬ' => 'ャ',
        'ｭ' => 'ュ',
        'ｮ' => 'ョ',
        'ｯ' => 'ッ',
        'ｱ' => 'ア',
        'ｲ' => 'イ',
        'ｳ' => 'ウ',
        'ｴ' => 'エ',
        'ｵ' => 'オ',
        'ｶ' => 'カ',
        'ｷ' => 'キ',
        'ｸ' => 'ク',
        'ｹ' => 'ケ',
        'ｺ' => 'コ',
        'ｻ' => 'サ',
        'ｼ' => 'シ',
        'ｽ' => 'ス',
        'ｾ' => 'セ',
        'ｿ' => 'ソ',
        'ﾀ' => 'タ',
        'ﾁ' => 'チ',
        'ﾂ' => 'ツ',
        'ﾃ' => 'テ',
        'ﾄ' => 'ト',
        'ﾅ' => 'ナ',
        'ﾆ' => 'ニ',
        'ﾇ' => 'ヌ',
        'ﾈ' => 'ネ',
        'ﾉ' => 'ノ',
        'ﾊ' => 'ハ',
        'ﾋ' => 'ヒ',
        'ﾌ' => 'フ',
        'ﾍ' => 'ヘ',
        'ﾎ' => 'ホ',
        'ﾏ' => 'マ',
        'ﾐ' => 'ミ',
        'ﾑ' => 'ム',
        'ﾒ' => 'メ',
        'ﾓ' => 'モ',
        'ﾔ' => 'ヤ',
        'ﾕ' => 'ユ',
        'ﾖ' => 'ヨ',
        'ﾗ' => 'ラ',
        'ﾘ' => 'リ',
        'ﾙ' => 'ル',
        'ﾚ' => 'レ',
        'ﾛ' => 'ロ',
        'ﾜ' => 'ワ',
        'ﾝ' => 'ン',
        _ => return None,
    };
    Some(mapped)
}

/// 全角カタカナ基底に濁点を合成（`カ`→`ガ`、`ウ`→`ヴ`）。濁点を取れない仮名は `None`。
fn compose_voiced(base: char) -> Option<char> {
    let v = match base {
        'ウ' => 'ヴ',
        'カ' => 'ガ',
        'キ' => 'ギ',
        'ク' => 'グ',
        'ケ' => 'ゲ',
        'コ' => 'ゴ',
        'サ' => 'ザ',
        'シ' => 'ジ',
        'ス' => 'ズ',
        'セ' => 'ゼ',
        'ソ' => 'ゾ',
        'タ' => 'ダ',
        'チ' => 'ヂ',
        'ツ' => 'ヅ',
        'テ' => 'デ',
        'ト' => 'ド',
        'ハ' => 'バ',
        'ヒ' => 'ビ',
        'フ' => 'ブ',
        'ヘ' => 'ベ',
        'ホ' => 'ボ',
        _ => return None,
    };
    Some(v)
}

/// 全角カタカナ基底に半濁点を合成（`ハ`→`パ`）。対象は は行のみ。
fn compose_semivoiced(base: char) -> Option<char> {
    let v = match base {
        'ハ' => 'パ',
        'ヒ' => 'ピ',
        'フ' => 'プ',
        'ヘ' => 'ペ',
        'ホ' => 'ポ',
        _ => return None,
    };
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::normalize_name;

    #[test]
    fn fullwidth_ascii_and_dot_to_halfwidth() {
        assert_eq!(normalize_name("Ｃ．ルメール".into()), "C.ルメール");
        assert_eq!(normalize_name("Ｄ．レーン２".into()), "D.レーン2");
    }

    #[test]
    fn halfwidth_kana_to_fullwidth() {
        // ｲｸｲﾉｯｸｽ → イクイノックス
        assert_eq!(normalize_name("ｲｸｲﾉｯｸｽ".into()), "イクイノックス");
    }

    #[test]
    fn composes_voiced_and_semivoiced() {
        assert_eq!(normalize_name("ｶﾞ".into()), "ガ");
        assert_eq!(normalize_name("ﾊﾟ".into()), "パ");
        assert_eq!(normalize_name("ｳﾞ".into()), "ヴ");
        // ﾀﾞｲﾜｽｶｰﾚｯﾄ → ダイワスカーレット
        assert_eq!(normalize_name("ﾀﾞｲﾜｽｶｰﾚｯﾄ".into()), "ダイワスカーレット");
    }

    #[test]
    fn trims_and_keeps_fullwidth_kana() {
        assert_eq!(normalize_name(" イクイノックス ".into()), "イクイノックス");
    }

    #[test]
    fn idempotent_on_already_normalized() {
        for s in ["イクイノックス", "C.ルメール", "ダイワスカーレット"] {
            assert_eq!(normalize_name(s.into()), s);
        }
    }

    #[test]
    fn dangling_voiced_mark_without_base_is_kept() {
        // 単独の濁点（基底なし）は合成できないのでそのまま残す（クラッシュしない）。
        let out = normalize_name("ﾞ".into());
        assert_eq!(out, "ﾞ");
    }
}
