//! スクレイパ共通ユーティリティ。JRA / netkeiba のレスポンス処理で重複しがちな
//! エンコーディング変換などを集約する。

use encoding_rs::{EUC_JP, Encoding};

/// EUC-JP のレスポンスボディを UTF-8 文字列へデコードする。
///
/// JRA は本文を EUC-JP で返すため、UTF-8 前提の `read_to_string` では
/// 「stream did not contain valid UTF-8」で失敗する。生バイトを受けてから本関数で
/// デコードする。charset が確実に EUC-JP な経路（JRA オッズページ等）で使う。
/// charset がレスポンスで変わりうる経路は [`decode_html`] を使う。
///
/// メンテ画面など別エンコーディングが混じると後段の token/table 不検出に化けて原因が
/// 見えにくいので、解釈できないバイトがあれば取得元 `context`（cname や URL）を添えて
/// 警告する。不正バイトは置換文字（U+FFFD）へ lossy 変換し panic しない。
pub fn decode_euc_jp(bytes: &[u8], context: &str) -> String {
    let (decoded, _, had_errors) = EUC_JP.decode(bytes);
    if had_errors {
        tracing::warn!(context, "response was not valid EUC-JP; parsing may fail");
    }
    decoded.into_owned()
}

/// `Content-Type` の charset ラベル（無ければ `None`）に従って HTML 本文をデコードする。
///
/// netkeiba はホストで本文エンコーディングが異なる: `race.netkeiba.com`（出馬表・結果・
/// オッズ）は `charset=UTF-8` を明示する一方、`db.netkeiba.com`（馬個別成績）は charset を
/// 返さず本文は EUC-JP。そのため charset 不明時は **EUC-JP にフォールバック**する。
/// EUC-JP 固定デコードは race.netkeiba.com の UTF-8 化で文字化けする回帰を起こしていた。
///
/// 解釈できないバイトがあれば取得元 `context` と採用エンコーディングを添えて警告し、
/// 不正バイトは置換文字（U+FFFD）へ lossy 変換して panic しない。
pub fn decode_html(bytes: &[u8], charset: Option<&str>, context: &str) -> String {
    let encoding = charset
        .and_then(|label| Encoding::for_label(label.trim().as_bytes()))
        .unwrap_or(EUC_JP);
    let (decoded, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        tracing::warn!(
            context,
            encoding = encoding.name(),
            "response body had decode errors; parsing may fail"
        );
    }
    decoded.into_owned()
}

/// `Content-Type` ヘッダ値から charset ラベルを取り出す（例: `text/html; charset=UTF-8` → `UTF-8`）。
/// charset 指定が無ければ `None`。値の前後空白と引用符は取り除く。
pub fn charset_from_content_type(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .filter_map(|part| part.split_once('='))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case("charset"))
        .map(|(_, value)| value.trim().trim_matches('"').to_string())
        .filter(|label| !label.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_euc_jp_body_without_utf8_error() {
        // 回帰ガード: JRA は EUC-JP を返すため、UTF-8 前提だと
        // 「stream did not contain valid UTF-8」で失敗していた（#104）。
        let (euc, _, had_errors) = encoding_rs::EUC_JP.encode("単勝・複勝オッズ");
        assert!(!had_errors, "test fixture must be encodable as EUC-JP");
        // バイト列は UTF-8 として不正であること（=旧経路が壊れていた条件）を確認する。
        assert!(std::str::from_utf8(&euc).is_err());
        assert_eq!(decode_euc_jp(&euc, "pwTAN001"), "単勝・複勝オッズ");
    }

    #[test]
    fn decode_euc_jp_is_lossy_for_invalid_bytes() {
        // メンテ画面等で EUC-JP として解釈できないバイトが来ても panic せず、
        // 置換文字へ lossy デコードする（had_errors=true の warn 経路）。
        let decoded = decode_euc_jp(&[0x80], "pwTAN001");
        assert!(decoded.contains('\u{FFFD}'));
    }

    #[test]
    fn decode_html_uses_utf8_when_header_declares_it() {
        // 回帰ガード: race.netkeiba.com は UTF-8 を明示する。EUC-JP 固定だと
        // 「芝1200m」等が文字化けして surface/distance パースが落ちていた（shutuba UTF-8 化）。
        let body = "芝1200m / 函館".as_bytes();
        assert_eq!(
            decode_html(body, Some("UTF-8"), "shutuba"),
            "芝1200m / 函館"
        );
    }

    #[test]
    fn decode_html_falls_back_to_euc_jp_without_charset() {
        // db.netkeiba.com（馬個別成績）は charset を返さず本文は EUC-JP。
        // charset 不明時は EUC-JP で解釈して正しく読めること。
        let (euc, _, had_errors) = EUC_JP.encode("競走成績");
        assert!(!had_errors);
        assert_eq!(decode_html(&euc, None, "horse/result"), "競走成績");
    }

    #[test]
    fn charset_from_content_type_extracts_label() {
        assert_eq!(
            charset_from_content_type("text/html; charset=UTF-8").as_deref(),
            Some("UTF-8")
        );
        assert_eq!(
            charset_from_content_type("text/html; charset=\"euc-jp\"").as_deref(),
            Some("euc-jp")
        );
        // charset 指定の無い db.netkeiba.com（text/html のみ）は None → 呼び出し側で EUC-JP。
        assert_eq!(charset_from_content_type("text/html"), None);
    }
}
