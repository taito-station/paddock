#!/usr/bin/env bash
# 締切前 prefetch（#237）と keep-awake（#264）の launchd エージェントを ~/Library/LaunchAgents/ に
# 配置して有効化する。各 plist テンプレートの __REPO_ROOT__ を実リポジトリパスへ置換してから load する。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEST_DIR="$HOME/Library/LaunchAgents"
# prefetch（オッズ取得）と keep-awake（スリープ抑止）の 2 エージェントをまとめて配置する。
LABELS=(com.paddock.prefetch-odds com.paddock.keep-awake)

mkdir -p "$DEST_DIR"
# 先に全テンプレートの存在を検証してから load を始める（片方 load 済みで他方欠落＝部分インストールを防ぐ）。
for LABEL in "${LABELS[@]}"; do
  [ -f "$SCRIPT_DIR/$LABEL.plist" ] || { echo "テンプレートが見つかりません: $SCRIPT_DIR/$LABEL.plist" >&2; exit 1; }
done
for LABEL in "${LABELS[@]}"; do
  TEMPLATE="$SCRIPT_DIR/$LABEL.plist"
  DEST="$DEST_DIR/$LABEL.plist"
  # __REPO_ROOT__ を実パスに置換して配置（| 区切りで sed に渡しパス中の / を気にしない）。
  sed "s|__REPO_ROOT__|$REPO_ROOT|g" "$TEMPLATE" > "$DEST"
  # 既存ロードがあれば外してから入れ直す（更新を確実に反映）。
  launchctl unload "$DEST" 2>/dev/null || true
  launchctl load "$DEST"
  echo "ロードしました: $DEST"
done

echo "確認: launchctl list | grep com.paddock"
echo "ログ: prefetch=/tmp/paddock-prefetch/logs/prefetch.log / keep-awake=/tmp/paddock-keep-awake/logs/keep-awake.log"
echo "注意: 当日カード（post_time 入り）は朝の paddock-fetch-card で投入済みであること。"
echo "      release バイナリ未ビルドなら: cargo build --release --bin paddock-fetch-card"
echo "取りこぼし確認: python3 scripts/predict-check/snapshot_coverage.py --date <YYYY-MM-DD>"
