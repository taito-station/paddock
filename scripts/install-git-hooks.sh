#!/usr/bin/env bash
# Git フックをリポジトリ管理下（scripts/git-hooks/）に配線する（#414）。
#
# core.hooksPath はローカル設定でコミットされないため、clone・並走 clone ごとに一度実行する。
# 相対パス（scripts/git-hooks）で張るので、同一 clone の各 worktree からも解決できる
# （core.hooksPath は worktree 間で共有される .git/config に入る）。
# 配線後は git が core.hooksPath のみを参照するため、旧 .git/hooks/pre-push は無効化される（二重実行なし）。
set -euo pipefail

# どこから呼ばれてもリポジトリルート基準で動くようにする。
cd "$(git rev-parse --show-toplevel)"

if [ ! -d scripts/git-hooks ]; then
  echo "[install-git-hooks] scripts/git-hooks が見つかりません。リポジトリルートで実行してください。" >&2
  exit 1
fi

# 実行ビットを確実に立てる（clone 時に失われた場合の保険。git は exec bit を追跡する）。
# 失敗しても致命ではない（コミット済みの 100755 を使う）が、無言化せず warn は出す。
chmod +x scripts/git-hooks/* 2>/dev/null \
  || echo "[install-git-hooks] warn: chmod に失敗（コミット済みの exec bit 100755 を使用）" >&2

git config core.hooksPath scripts/git-hooks

echo "[install-git-hooks] core.hooksPath = $(git config --get core.hooksPath)"
echo "[install-git-hooks] 配線済みフック: $(ls scripts/git-hooks | tr '\n' ' ')"
