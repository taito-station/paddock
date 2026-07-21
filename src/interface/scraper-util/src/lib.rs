//! スクレイパ共通ユーティリティ。JRA / netkeiba のレスポンス処理で重複しがちな
//! エンコーディング変換やリトライポリシーなどを集約する。

use std::time::Duration;

use encoding_rs::{EUC_JP, Encoding};

/// 一時障害に対する総試行回数（初回 1 + リトライ 2）。ADR 0021/0029 で JRA に導入した
/// policy を netkeiba（#288）にも展開し、ここへ一元化した（#460）。
const MAX_ATTEMPTS: u32 = 3;
/// 指数バックオフの基準値。試行 N は `BASE * 2^(N-1)`（1s, 2s, …）だけ待つ。
const RETRY_BASE_BACKOFF: Duration = Duration::from_secs(1);

/// 再試行する価値のある一時障害かを判定する。transport hiccup（接続リセット os error 54 =
/// `Io`・タイムアウト・接続失敗・名前解決失敗・不完全 HTTP 応答）と 5xx を transient とし、
/// 4xx（JRA の 403/404 "absent" 応答を含む）とリクエスト不正は非 transient（即返し）とする。
///
/// 4xx の扱い差分（JRA は 403/404 を absent に、netkeiba は即エラー）は本判定の外側、
/// 各呼び出し元のレスポンス処理で注入する。ここでは「リトライすべきか」だけを一元判定する。
pub fn is_transient(err: &ureq::Error) -> bool {
    match err {
        ureq::Error::Timeout(_)
        | ureq::Error::Io(_)
        | ureq::Error::ConnectionFailed
        | ureq::Error::HostNotFound
        | ureq::Error::Protocol(_) => true,
        ureq::Error::StatusCode(code) => *code >= 500,
        _ => false,
    }
}

/// `attempt_fn` を実行し、transient 失敗時は指数バックオフ（1s/2s）で最大 [`MAX_ATTEMPTS`]
/// 回まで再試行する共通リトライループ（ADR 0021/0029・#288 を #460 で統合）。
///
/// リトライ回数・バックオフ・transient 判定・warn ログ形式をここへ一元化し、JRA / netkeiba
/// 双方が同一ポリシーを共有する。各呼び出し元は `attempt_fn` に「1 回分の GET」（UA 付与・
/// レート制御・ボディ抽出など経路固有の処理）を渡し、`url` は warn ログのラベルに使う。
///
/// レスポンスヘッダ取得（`.call()` 相当）のみを再試行する用途を想定する。ボディ読取中の失敗を
/// リトライしたくない場合は、`attempt_fn` 側でヘッダ取得だけを行い、ボディ読取は本関数の外で行う。
pub fn call_with_retry<T>(
    url: &str,
    mut attempt_fn: impl FnMut() -> Result<T, ureq::Error>,
) -> Result<T, ureq::Error> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match attempt_fn() {
            Ok(value) => return Ok(value),
            Err(err) if attempt < MAX_ATTEMPTS && is_transient(&err) => {
                // saturating で MAX_ATTEMPTS を増やしても shift/乗算が panic しないようにする。
                let backoff = RETRY_BASE_BACKOFF.saturating_mul(2u32.saturating_pow(attempt - 1));
                tracing::warn!(
                    url,
                    attempt,
                    max_attempts = MAX_ATTEMPTS,
                    backoff_ms = backoff.as_millis() as u64,
                    error = %err,
                    "transient fetch error; retrying after backoff"
                );
                std::thread::sleep(backoff);
            }
            Err(err) => return Err(err),
        }
    }
}

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
/// netkeiba はホストで本文エンコーディングが異なる: `race.netkeiba.com`（出馬表・結果）は
/// `charset=UTF-8` を明示する一方、`db.netkeiba.com`（馬個別成績）は charset を返さず本文は
/// EUC-JP。そのため charset 不明時は **EUC-JP にフォールバック**する。EUC-JP 固定デコードは
/// race.netkeiba.com の UTF-8 化で文字化けする回帰を起こしていた。
/// （オッズ API は UTF-8 JSON で本関数を通らず別経路でデコードする。）
///
/// charset ラベルは付くが `encoding_rs` が解決できない（未知・タイポ等）場合は、無言で
/// EUC-JP に倒すと UTF-8 本文を化けさせる同種の回帰を見落とすため、`context` とラベルを
/// 添えて警告してからフォールバックする。解釈できないバイトがあっても置換文字（U+FFFD）へ
/// lossy 変換して panic しない。
pub fn decode_html(bytes: &[u8], charset: Option<&str>, context: &str) -> String {
    let label = charset.map(str::trim).filter(|label| !label.is_empty());
    let encoding = match label {
        Some(label) => Encoding::for_label(label.as_bytes()).unwrap_or_else(|| {
            tracing::warn!(
                context,
                charset = label,
                "未知の charset ラベル。EUC-JP にフォールバックする（誤デコードの可能性）"
            );
            EUC_JP
        }),
        None => EUC_JP,
    };
    let (decoded, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        // charset 宣言の有無も添える。宣言なし(=EUC-JP フォールバック)で decode error が
        // 出るのは、charset を返さない経路が将来 UTF-8 化した兆候でありうるため切り分け用。
        tracing::warn!(
            context,
            encoding = encoding.name(),
            charset_declared = label.is_some(),
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
    use std::cell::Cell;
    use std::time::Instant;

    use super::*;

    #[test]
    fn is_transient_classifies_retryable_errors() {
        // 5xx and transport hiccups are retried.
        assert!(is_transient(&ureq::Error::StatusCode(500)));
        assert!(is_transient(&ureq::Error::StatusCode(503)));
        assert!(is_transient(&ureq::Error::ConnectionFailed));
        assert!(is_transient(&ureq::Error::HostNotFound));
        assert!(is_transient(&ureq::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset"
        ))));
        // 4xx (including the "absent" 403/404) and client mistakes are not.
        assert!(!is_transient(&ureq::Error::StatusCode(404)));
        assert!(!is_transient(&ureq::Error::StatusCode(403)));
        assert!(!is_transient(&ureq::Error::StatusCode(400)));
        assert!(!is_transient(&ureq::Error::BadUri("nope".into())));
    }

    #[test]
    fn call_with_retry_retries_transient_then_succeeds() {
        // 503, 503, 200 → 3 回試行して成功。バックオフ（1s + 2s = 3s）分だけ待つ。
        let attempts = Cell::new(0u32);
        let start = Instant::now();
        let result: Result<&str, ureq::Error> = call_with_retry("http://test/x", || {
            let n = attempts.get();
            attempts.set(n + 1);
            if n < 2 {
                Err(ureq::Error::StatusCode(503))
            } else {
                Ok("ok")
            }
        });
        assert_eq!(result.unwrap(), "ok");
        assert_eq!(attempts.get(), 3, "2 リトライ後に成功するはず");
        assert!(
            start.elapsed() >= Duration::from_secs(3),
            "1s + 2s のバックオフを待つはず、実測 {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn call_with_retry_gives_up_after_max_attempts() {
        // 恒常 5xx → MAX_ATTEMPTS 回ちょうどで打ち切り、最後のエラーを返す（無限ループしない）。
        let attempts = Cell::new(0u32);
        let result: Result<&str, ureq::Error> = call_with_retry("http://test/x", || {
            attempts.set(attempts.get() + 1);
            Err(ureq::Error::StatusCode(503))
        });
        assert!(result.is_err(), "恒常 5xx はエラーとして返る");
        assert_eq!(
            attempts.get(),
            MAX_ATTEMPTS,
            "MAX_ATTEMPTS 回ちょうど試行して諦めるはず"
        );
    }

    #[test]
    fn call_with_retry_does_not_retry_non_transient() {
        // 404（非 transient）は即返し。リトライしない。
        let attempts = Cell::new(0u32);
        let result: Result<&str, ureq::Error> = call_with_retry("http://test/x", || {
            attempts.set(attempts.get() + 1);
            Err(ureq::Error::StatusCode(404))
        });
        assert!(result.is_err());
        assert_eq!(attempts.get(), 1, "404 は 1 回で即返し（リトライ無し）");
    }

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
    fn decode_html_falls_back_to_euc_jp_for_unresolvable_label() {
        // charset ラベルは付くが encoding_rs が解決できない（未知）場合も EUC-JP に倒す
        // （warn を出した上でのフォールバック。挙動として EUC-JP デコードになることを確認）。
        let (euc, _, _) = EUC_JP.encode("競走成績");
        assert_eq!(
            decode_html(&euc, Some("not-a-real-charset"), "horse/result"),
            "競走成績"
        );
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
