use crate::normalize::normalize_name;
use crate::string::define_string;

// 馬名。取り込み（成績/出馬表パース）と検索（analyze）で同一表現に揃えるため、
// 騎手名と同じ正規化（全角英数→半角・半角カナ→全角・trim）を適用する。
define_string!(HorseName, max = 30, normalize = normalize_name);

#[cfg(test)]
mod tests {
    use super::HorseName;

    #[test]
    fn normalizes_halfwidth_kana_to_fullwidth() {
        let h = HorseName::try_from("ｲｸｲﾉｯｸｽ").unwrap();
        assert_eq!(h.value(), "イクイノックス");
    }

    #[test]
    fn fullwidth_and_halfwidth_input_match() {
        let full = HorseName::try_from("イクイノックス").unwrap();
        let half = HorseName::try_from("ｲｸｲﾉｯｸｽ").unwrap();
        assert_eq!(full.value(), half.value());
    }
}
