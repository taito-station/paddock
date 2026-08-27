# 636 — 変数直後の全角文字が識別子に取り込まれ `set -u` で落ちる（生資料）

2026-08-16 の開催後片付けで `deployments/launchd/uninstall.sh` が異常終了した実測。
issue 本文は [gh issue view 636](https://github.com/taito-station/paddock/issues/636)。

質問票: [QA-fullwidth-after-var-636.md](../qa/QA-fullwidth-after-var-636.md)
蒸留先: [ci-pipeline.md](../knowledge/ci-pipeline.md) / `scripts/check-shell-var-nonascii.sh` のヘッダ。

## 起きたこと

```
/Users/ito-taito/workspace/paddock/deployments/launchd/uninstall.sh: line 36: pid?: unbound variable
```

該当行:

```sh
kill "$pid" 2>/dev/null && echo "keep-awake の caffeinate を停止しました（pid $pid）"
```

bash は識別子の終端判定に `isalnum()` を使う。これがロケールのテーブルを参照するため、UTF-8 ロケールでは
全角 `）` のバイト（`EF BC 89`）まで変数名に取り込まれ、`pid）` という未定義変数になる。

## ロケール依存であってバージョン依存ではない（実測）

macOS 24.6.0 / GNU bash 3.2.57(1)-release (x86_64-apple-darwin24):

| ロケール | `bash -c 'set -u; v=abc; echo "x $v）y"'` |
|---|---|
| `LC_ALL=C` | **正常**（`x abc）y`） |
| `LC_ALL=en_US.UTF-8` | `bash: v?: unbound variable` |
| `LC_ALL=ja_JP.UTF-8` | 同上 |
| `LC_ALL=C.UTF-8` | 同上 |

**Linux / glibc での挙動は未確認。** colima の VM が I/O エラー（`/bin/bash: Input/output error`）で
使用不能だったため測れていない。glibc の C.UTF-8 は 0x7F 超のバイトを `isalnum` にしない可能性があり、
その場合 **CI（ubuntu）は緑のまま macOS だけ壊れる**ことになる。いずれにせよ対策は静的検査で、
実行時の挙動に依存しない形にしてある（後述）。

## 露出が「人が叩いたとき」に偏る構造

launchd の plist は `EnvironmentVariables` に **`PATH` しか設定していない**（LANG なし）。
つまり launchd 経由のジョブは C ロケールで動くので落ちない。

その証拠に `scripts/verify-backup-restore.sh:169` は**通常系**（サイドカーが有効なら必ず通る）で
同じ地雷を踏んでいたが、常駐ジョブ `com.paddock.verify-backup-restore` の直近 exit は 0 だった。

**壊れるのは人が UTF-8 の端末から叩いたときだけ。** 開催日の運用スクリプトはほぼそれに当たる。

## 実害の出方（部分実行で止まる）

`uninstall.sh:36` は `kill "$pid" && echo "..."` の形なので、**`kill` は成功してから `echo` の展開で落ちる**。
**`set -u` の未定義変数展開は非対話シェルを即座に終了させる**（`set -e` の話ではない。`set -e` は
AND リストの最終コマンド以外の失敗では中断しないので、仮に `kill` が失敗しても中断はしない）。
その致命的終了により、次行の `rm -rf "$LOCK_DIR"` に到達しない。

2026-08-16 の実行結果:

| 処理 | 結果 |
|---|---|
| launchd 3 本の除去（line 36 より前） | 成功 |
| caffeinate の停止（`kill`） | 成功 |
| `/tmp/paddock-keep-awake.lock.d` の削除 | **未実行**（手動で消した） |

「最後まで走ったように見えて走っていない」形になる。

## 走査結果（12 箇所 / 8 ファイル）

`git ls-files '*.sh' scripts/mdq scripts/git-hooks/pre-push` を対象に、
ブレース無しの `$name` の直後が非 ASCII の箇所を数えた。

**展開されるもの（9 箇所 / 6 ファイル）**:

| 箇所 | 到達経路 |
|---|---|
| `deployments/launchd/uninstall.sh:36` | 通常系（今回の実害） |
| `scripts/verify-backup-restore.sh:169` | 通常系（launchd 経由は C ロケールで難を逃れていた） |
| `scripts/predict-check/refresh_ev.sh:186` | 通常系 |
| `scripts/predict-check/refresh_ev.sh:182` | 失敗時（`wide FAIL`） |
| `scripts/seed-db.sh:106` | 失敗時（`pg_dump に失敗`） |
| `scripts/test-check-vendored-swagger.sh:73`（2 箇所） | 失敗時（`✗`） |
| `scripts/test-db-guards.sh:43` / `:46` | 失敗時（`NG`） |

**展開されないもの（3 箇所）**: `deployments/launchd/install.sh:7`（行頭コメント）/
`scripts/seed-db.sh:22`（`cat <<'EOF'` のクォート付きヒアドキュメント）/
`scripts/test-check-adr-numbers.sh:102`（行頭コメント。**この罠を説明する悪い例**）。

> **失敗報告の行に偏っているのが最悪の性質。** 6 箇所が `✗` `NG` `FAIL` を出す分岐にある。
> 何が失敗したかを伝えるはずのメッセージが、まさにその場面で消える。

## `shellcheck` は検出しない（実測）

`shellcheck 0.11.0` に以下を通しても `--severity=style` で exit 0:

```sh
#!/usr/bin/env bash
set -euo pipefail
v=abc
echo "x $v）y"
```

## 既に警告が書かれていた

`scripts/test-check-adr-numbers.sh:102` に、この罠についてのコメントが**着手前から存在した**。

> 変数の直後に全角文字が続く箇所は必ずブレースで閉じる。`$label（` と書くと bash が
> 全角括弧の UTF-8 バイトまで識別子の一部として読み、unbound variable で落ちる。

把握済みの知識が 1 ファイルのコメントに留まり、他へ効いていなかった。
ADR 0073 の「人手の規律に委ねない」対象そのもの。

## 対策の形

**静的検査**にした（`scripts/check-shell-var-nonascii.sh`）。実行時の挙動はロケールとプラットフォームに
依存して確かめにくく、回帰テストを実挙動に依存させると環境ごとに結果が変わるため、
**字面で禁じて回帰テストも字面判定だけで完結させる**。

## 既知の非カバー範囲（レビューで判明）

**展開される複数行文脈の `#` 始まり行は素通りする。** 行頭コメント除外が「その行が本当にコメントか」を
見ていないため、クォート無しヒアドキュメント本文・複数行ダブルクォート文字列の継続行・`$(...)` の中が
該当する。いずれも展開されるので実際には落ちる:

```
$ LC_ALL=ja_JP.UTF-8 bash -c 'set -u; title=x; cat <<EOF
# $title（説明）
EOF'
bash: title?: unbound variable        # LC_ALL=C なら正常
```

`scripts/test-check-adr-numbers.sh` の `write_adr()` が `cat >"$path" <<EOF` の直下に `# $title` を
持っており、**現状は `$title` の直後が改行なので無害なだけ**。一字足せば検査を素通りして落ちる。
本 PR ではこの箇所をブレース化して地雷を除いたうえで、穴自体は検査スクリプトのヘッダに明記した
（ヒアドキュメントの追跡には状態機械が要り、検査の単純さと引き換えになるため見送り）。

**`.github/workflows/*.yml` の `run:` と `deployments/*.Dockerfile` の `RUN`** も UTF-8 ロケールの
bash で走るが対象外（走査したところ現時点で違反 0 件）。
