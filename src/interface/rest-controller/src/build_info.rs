//! ビルド時に埋め込んだ世代情報（git sha / ビルド時刻）へのアクセス（#570）。
//! 値は `build.rs` が `cargo:rustc-env` で注入する。`/api/health` と起動ログの双方で使う。

use chrono::{DateTime, SecondsFormat};

/// ビルド元の git sha（短縮）。未コミット変更ありのビルドは `-dirty` 付き。`.git` 不在時は `unknown`。
pub const GIT_SHA: &str = env!("PADDOCK_GIT_SHA");

/// ビルド時刻（UNIX epoch 秒の文字列）。
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
