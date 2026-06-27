#!/usr/bin/env bash
# 締切前 prefetch（#237）の launchd エージェントを ~/Library/LaunchAgents/ に配置して有効化する。
# plist テンプレートの __REPO_ROOT__ を実リポジトリパスへ置換してから load する。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
LABEL="com.paddock.prefetch-odds"
TEMPLATE="$SCRIPT_DIR/$LABEL.plist"
DEST_DIR="$HOME/Library/LaunchAgents"
DEST="$DEST_DIR/$LABEL.plist"

[ -f "$TEMPLATE" ] || { echo "テンプレートが見つかりません: $TEMPLATE" >&2; exit 1; }
mkdir -p "$DEST_DIR"

# __REPO_ROOT__ を実パスに置換して配置（| 区切りで sed に渡しパス中の / を気にしない）。
sed "s|__REPO_ROOT__|$REPO_ROOT|g" "$TEMPLATE" > "$DEST"

# 既存ロードがあれば外してから入れ直す（更新を確実に反映）。
launchctl unload "$DEST" 2>/dev/null || true
launchctl load "$DEST"

echo "ロードしました: $DEST"
echo "確認: launchctl list | grep $LABEL"
echo "ログ: $REPO_ROOT/.launchd-prefetch.out.log（prefetch 本体ログは \$TMPDIR/paddock-prefetch/logs/prefetch.log）"
echo "注意: 当日カード（post_time 入り）は朝の paddock-fetch-card で投入済みであること。"
echo "      release バイナリ未ビルドなら: cargo build --release --bin paddock-fetch-card"
