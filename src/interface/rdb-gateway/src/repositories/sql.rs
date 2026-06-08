//! stats クエリ共通の SQL 片ヘルパ。

/// `as_of` カットオフ用の `AND races.date < <placeholder>` 述語片を返す。
///
/// `cutoff` が `None`（全期間集計）なら空文字。日付値は呼び出し側がプレースホルダで
/// バインドするため、ここでは文字列連結に値を含めない（SQL インジェクション安全）。
pub(crate) fn date_lt_pred(cutoff: Option<&str>, placeholder: &str) -> String {
    if cutoff.is_some() {
        format!("AND races.date < {placeholder}")
    } else {
        String::new()
    }
}
