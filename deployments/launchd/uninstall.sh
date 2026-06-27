#!/usr/bin/env bash
# 締切前 prefetch（#237）の launchd エージェントを停止・除去する。
set -euo pipefail

LABEL="com.paddock.prefetch-odds"
DEST="$HOME/Library/LaunchAgents/$LABEL.plist"

if [ -f "$DEST" ]; then
  launchctl unload "$DEST" 2>/dev/null || true
  rm -f "$DEST"
  echo "除去しました: $DEST"
else
  echo "未インストール: $DEST は存在しません"
fi
