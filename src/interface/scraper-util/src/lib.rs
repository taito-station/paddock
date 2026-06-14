//! スクレイパ共通ユーティリティ。JRA / netkeiba のレスポンス処理で重複しがちな
//! エンコーディング変換などを集約する。

/// EUC-JP のレスポンスボディを UTF-8 文字列へデコードする。
///
/// JRA / netkeiba は本文を EUC-JP で返すため、UTF-8 前提の `read_to_string` では
/// 「stream did not contain valid UTF-8」で失敗する。生バイトを受けてから本関数で
/// デコードする。
///
/// メンテ画面など別エンコーディングが混じると後段の token/table 不検出に化けて原因が
/// 見えにくいので、解釈できないバイトがあれば取得元 `context`（cname や URL）を添えて
/// 警告する。不正バイトは置換文字（U+FFFD）へ lossy 変換し panic しない。
pub fn decode_euc_jp(bytes: &[u8], context: &str) -> String {
    let (decoded, _, had_errors) = encoding_rs::EUC_JP.decode(bytes);
    if had_errors {
        tracing::warn!(context, "response was not valid EUC-JP; parsing may fail");
    }
    decoded.into_owned()
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
}
