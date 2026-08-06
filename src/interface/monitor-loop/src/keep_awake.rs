//! 監視プロセス自身によるアイドルスリープ抑止（#568）。
//!
//! 終日バックグラウンドで回る監視（predict-watch / odds-collect）は、ホストが寝ると止まる。
//! 復帰後の自動再開は [`crate::driver`] の wall-clock 待機で担保しているが、**寝ている間の
//! スイープそのものは取り返せない**（発走が過ぎたレースは二度と評価できない）。そこで監視中は
//! ホストを起こしたままにする。
//!
//! ## 既存の keep-awake エージェント（#264）との棲み分け
//!
//! `deployments/launchd/com.paddock.keep-awake.plist` ＋ `scripts/predict-check/keep_awake.sh` は
//! **締切前 prefetch（#237）の launchd タイマー**を回すための常駐ジョブで、開催日の朝に
//! `install.sh` を叩く運用が要る。監視プロセスの生存期間とは無関係なので、install 忘れがあれば
//! 監視は無防備になる。ここでは**監視プロセスが自分の生存期間だけ**抑止を確保し、運用手順への
//! 依存を外す（#264 の launchd 側はそのまま prefetch 用に残す）。
//!
//! ## 限界（best-effort）
//!
//! `caffeinate -i` が止められるのは**アイドルスリープだけ**。クラムシェル（蓋閉じ）スリープや
//! `pmset` のスケジュールスリープは止められず、既に寝ているホストを起こすこともできない。
//! 完全な堅牢化は常時稼働ホストへの移設（`deployments/launchd/README.md`）。

use std::process::Child;

/// `caffeinate` の絶対パス。PATH 解決にすると、書き込み可能なディレクトリが PATH 前方にある環境で
/// 監視プロセスの権限・環境変数（DB URL 等）を継承した別バイナリが起動しうる。常駐プロセスから
/// 外部コマンドを spawn するので固定パスにする。
#[cfg(target_os = "macos")]
const CAFFEINATE: &str = "/usr/bin/caffeinate";

/// 監視中のアイドルスリープ抑止を確保する（macOS のみ・best-effort）。
///
/// 返り値の [`Child`] は `caffeinate -i -w <自分の pid>`。**自プロセスを見張らせる**ことで、
/// 監視がどう終わっても（正常終了・パニック・kill）抑止が解放される＝解放忘れが構造上ない。
///
/// 解放を担うのは `-w` の pid 監視であって、この [`Child`] の drop ではない（`std` の `Child` は
/// drop で kill も wait もしない）。呼び出し側が返り値を保持するのは「監視中は確保したままにする」
/// という意図の表明で、早く drop しても抑止は続く。逆に caffeinate が外部から kill された場合は
/// 抑止だけが静かに失われる（wait していないのでゾンビが 1 つ残るが、監視の終了で回収される）。
///
/// 確保できなければ `None`。macOS 以外、または `caffeinate` 不在の環境では何もしない。
#[cfg(target_os = "macos")]
pub fn acquire() -> Option<Child> {
    use std::process::{Command, Stdio};

    let pid = std::process::id().to_string();
    match Command::new(CAFFEINATE)
        .args(["-i", "-w", &pid])
        // caffeinate は環境変数を必要としない。DB 接続 URL 等を子へ渡す理由が無いので落とす。
        .env_clear()
        // 監視の標準出力に caffeinate の出力を混ぜない。stdin も切って端末を掴ませない。
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            println!(
                "── アイドルスリープ抑止を確保しました（caffeinate -i -w {pid}）。監視の終了で自動解放されます。\
                 蓋閉じ / pmset スケジュールスリープは抑止できません。"
            );
            Some(child)
        }
        Err(e) => {
            // 沈黙させない: 抑止が無い状態で寝られると監視が途切れ、それが「妙味なし」と誤読される。
            println!(
                "⚠ アイドルスリープ抑止を確保できませんでした（caffeinate 起動失敗: {e}）。\
                 ホストがスリープすると監視が途切れます（復帰後は自動再開しますが、寝ている間のスイープは失われます）。"
            );
            None
        }
    }
}

/// macOS 以外は抑止手段を持たない（no-op）。CI（Linux）でもビルドが通るようにするための分岐。
#[cfg(not(target_os = "macos"))]
pub fn acquire() -> Option<Child> {
    None
}
