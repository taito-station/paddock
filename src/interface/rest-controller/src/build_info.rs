//! ビルド時に埋め込んだ世代情報（git sha / ビルド時刻）へのアクセス（#570）。
//! 値は `build.rs` が `cargo:rustc-env` で注入する。`/api/health` と起動ログの双方で使う。

use chrono::{DateTime, SecondsFormat};

/// ビルド元の git sha（短縮）。未コミット変更ありのビルドは `-dirty` 付き。`.git` 不在時は `unknown`。
pub const GIT_SHA: &str = env!("PADDOCK_GIT_SHA");

/// ビルド時刻（UNIX epoch 秒の文字列）。厳密には build.rs が最後に実行された時刻＝概ねビルド時刻。
/// HEAD が動くと build.rs が再実行され更新されるため、sha が同じでも再ビルドで進みうる
/// （バイナリの厳密なリンク時刻とは一致しないことがある）。世代識別の主キーは [`GIT_SHA`]。
const BUILD_EPOCH: &str = env!("PADDOCK_BUILD_EPOCH");

/// ビルド時刻を UTC の rfc3339（秒精度）で返す。パースできなければ `unknown`。
pub fn build_time_rfc3339() -> String {
    BUILD_EPOCH
        .parse::<i64>()
        .ok()
        .and_then(|secs| DateTime::from_timestamp(secs, 0))
        .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_else(|| "unknown".to_string())
}
