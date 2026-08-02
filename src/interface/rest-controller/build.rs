//! ビルド時に git sha とビルド時刻を環境変数として埋め込む（#570）。
//!
//! 稼働中の `paddock-api` が「どの世代の成果物か」を `/api/health` で自己申告できるようにする。
//! 外部 crate（vergen 等）は使わず `git` CLI + std だけで完結させる。`.git` が無い環境
//! （Docker ビルド等）では sha を `unknown` にフォールバックする。
use std::path::Path;
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

/// 存在するパスだけ `rerun-if-changed` に登録する。存在しないパスを指定すると cargo が
/// 「常に変化した」とみなし毎ビルド build.rs を再実行する churn を招くため、実在チェックを挟む。
fn watch(path: &str) {
    if Path::new(path).exists() {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn main() {
    // git sha（短縮）。取得できなければ unknown。
    let mut sha = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    // 追跡ファイルに未コミット変更があれば `-dirty` を付す（＝コミット済み HEAD と一致しない
    // バイナリを検知できるようにする）。未追跡ファイル（ビルド生成物・作業メモ等）は世代ずれと
    // 無関係なので `--untracked-files=no` で除外する。git() は空文字（＝クリーン）も status 取得
    // 失敗も None にするため、この分岐は「追跡変更あり」のときだけ通る（取得失敗時は dirty 判定
    // 不能につきクリーン扱い＝何も足さない）。
    if git(&["status", "--porcelain", "--untracked-files=no"]).is_some() {
        sha.push_str("-dirty");
    }

    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    println!("cargo:rustc-env=PADDOCK_GIT_SHA={sha}");
    println!("cargo:rustc-env=PADDOCK_BUILD_EPOCH={epoch}");

    // HEAD の位置が動いたら（commit / checkout / reset / amend）build.rs を再実行して sha を更新する。
    // worktree ではファイルの所在が per-worktree gitdir と共有 common dir に分かれるため、両方を見る:
    //   - HEAD / logs/HEAD は per-worktree（`--absolute-git-dir`）
    //   - packed-refs / refs/heads/<branch> は共有側（`--git-common-dir`）
    // commit は loose ref `refs/heads/<branch>` を更新する（HEAD は symref なので checkout でのみ変わる）。
    // 参照が packed 済みなら loose ref は無く packed-refs 側が動く。reflog（logs/HEAD）は保険。
    // watch() が実在するものだけ登録するので、状態（loose/packed）に応じて有効な物のみ拾える。
    if let Some(gitdir) = git(&["rev-parse", "--absolute-git-dir"]) {
        watch(&format!("{gitdir}/HEAD"));
        watch(&format!("{gitdir}/logs/HEAD"));
    }
    let common_dir = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .or_else(|| git(&["rev-parse", "--absolute-git-dir"]));
    if let Some(common_dir) = common_dir {
        watch(&format!("{common_dir}/packed-refs"));
        // symbolic-ref は `refs/heads/<branch>` を返す（detached HEAD なら None＝loose ref 無し）。
        if let Some(head_ref) = git(&["symbolic-ref", "--quiet", "HEAD"]) {
            watch(&format!("{common_dir}/{head_ref}"));
        }
    }
}
