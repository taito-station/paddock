//! ビルド時に git sha とビルド時刻を環境変数として埋め込む（#570）。
//!
//! 稼働中の `paddock-api` が「どの世代の成果物か」を `/api/health` で自己申告できるようにする。
//! 外部 crate（vergen 等）は使わず `git` CLI + std だけで完結させる。`.git` が無い環境
//! （Docker ビルド等）では sha を `unknown` にフォールバックする。
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// `git <args>` を実行し、成功かつ非空なら trim した stdout を返す。
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn main() {
    // git sha（短縮）。取得できなければ unknown。作業ツリーに未コミット変更があれば `-dirty` を付す
    // （＝コミット済み HEAD と一致しないバイナリを検知できるようにする）。
    let mut sha = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    if git(&["status", "--porcelain"]).is_some() {
        // 非空（＝汚れている）ときだけ Some が返る（git() は空文字を None にするため）。
        sha.push_str("-dirty");
    }

    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    println!("cargo:rustc-env=PADDOCK_GIT_SHA={sha}");
    println!("cargo:rustc-env=PADDOCK_BUILD_EPOCH={epoch}");

    // HEAD の位置が動いたら（commit / checkout / reset）このビルドスクリプトを再実行して sha を更新する。
    // logs/HEAD は参照移動のたびに追記されるため、commit も branch 切替も 1 つで捕捉できる。
    if let Some(gitdir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={gitdir}/HEAD");
        println!("cargo:rerun-if-changed={gitdir}/logs/HEAD");
        println!("cargo:rerun-if-changed={gitdir}/packed-refs");
    }
}
