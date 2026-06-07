use crate::string::define_string;

define_string!(JockeyName, max = 30, normalize = normalize_jockey);

/// 騎手名を正規化する。全角英字・数字を半角へ、全角ピリオド `．`(U+FF0E) を `.` へ変換し、
/// 前後の空白を除去する。これにより `Ｃ．ルメール` と `C.ルメール` が同一表現になり、
/// 取り込み（成績/出馬表パース）と `analyze jockey` の検索入力が揃って完全一致で引ける。
fn normalize_jockey(value: String) -> String {
    let mapped: String = value
        .chars()
        .map(|c| match c {
            'Ａ'..='Ｚ' => (b'A' + (c as u32 - 'Ａ' as u32) as u8) as char,
            'ａ'..='ｚ' => (b'a' + (c as u32 - 'ａ' as u32) as u8) as char,
            '０'..='９' => (b'0' + (c as u32 - '０' as u32) as u8) as char,
            '．' => '.',
            other => other,
        })
        .collect();
    mapped.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::JockeyName;

    #[test]
    fn normalizes_fullwidth_latin_and_dot() {
        let j = JockeyName::try_from("Ｃ．ルメール").unwrap();
        assert_eq!(j.value(), "C.ルメール");
    }

    #[test]
    fn halfwidth_and_fullwidth_input_match() {
        let full = JockeyName::try_from("Ｃ．ルメール").unwrap();
        let half = JockeyName::try_from("C.ルメール").unwrap();
        assert_eq!(full.value(), half.value());
    }

    #[test]
    fn trims_surrounding_space_and_keeps_japanese() {
        let j = JockeyName::try_from(" 横山和生 ").unwrap();
        assert_eq!(j.value(), "横山和生");
    }

    #[test]
    fn normalizes_fullwidth_digits() {
        // 万一数字が混じっても半角化する（純数字は別途パーサ側で除外）。
        let j = JockeyName::try_from("Ｄ．レーン２").unwrap();
        assert_eq!(j.value(), "D.レーン2");
    }
}
