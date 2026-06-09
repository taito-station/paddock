use crate::normalize::normalize_name;
use crate::string::define_string;

// 騎手名。馬名と共通の正規化（全角英数→半角・全角ピリオド→`.`・半角カナ→全角・trim）を適用し、
// `Ｃ．ルメール` と `C.ルメール` が同一表現になって取り込みと検索が揃って引ける。
define_string!(JockeyName, max = 30, normalize = normalize_name);

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
        let j = JockeyName::try_from("Ｄ．レーン２").unwrap();
        assert_eq!(j.value(), "D.レーン2");
    }
}
