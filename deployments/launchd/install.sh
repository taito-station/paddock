#!/usr/bin/env bash
# 締切前 prefetch（#237）・keep-awake（#264）・日次 DB バックアップ（#265）・
# バックアップ鮮度監視（#490）の launchd エージェントを ~/Library/LaunchAgents/ に配置して有効化する。
# 各 plist テンプレートの __REPO_ROOT__（リポジトリパス）と
# __HOME__（$HOME・ログ出力先。backup-db / backup-staleness が使用）を実値へ置換してから load する。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEST_DIR="$HOME/Library/LaunchAgents"
# prefetch（オッズ取得）・keep-awake（スリープ抑止）・backup-db（日次 DB バックアップ）・
# backup-staleness（鮮度監視）の 4 エージェントをまとめて配置する。backup-db / backup-staleness は
# 常駐で、開催日限定の uninstall では外さない
# （uninstall.sh は prefetch/keep-awake のみ除去する。#416 で二重規約を解消し install に統合）。
LABELS=(com.paddock.prefetch-odds com.paddock.keep-awake com.paddock.backup-db com.paddock.backup-staleness)

mkdir -p "$DEST_DIR"
# 先に全テンプレートの存在を検証してから load を始める（片方 load 済みで他方欠落＝部分インストールを防ぐ）。
for LABEL in "${LABELS[@]}"; do
  [ -f "$SCRIPT_DIR/$LABEL.plist" ] || { echo "テンプレートが見つかりません: $SCRIPT_DIR/$LABEL.plist" >&2; exit 1; }
done
for LABEL in "${LABELS[@]}"; do
  TEMPLATE="$SCRIPT_DIR/$LABEL.plist"
  DEST="$DEST_DIR/$LABEL.plist"
  # __REPO_ROOT__ / __HOME__ を実パスに置換して配置（| 区切りで sed に渡しパス中の / を気にしない）。
  # __HOME__ は backup-db / backup-staleness plist が持ち、他 2 つには無いので -e 追加は no-op（無害）。
  sed -e "s|__REPO_ROOT__|$REPO_ROOT|g" -e "s|__HOME__|$HOME|g" "$TEMPLATE" > "$DEST"
  # 既存ロードがあれば外してから入れ直す（更新を確実に反映）。配置は legacy の load/unload を
  # 意図的に使う（unload→load は再ロードで冪等。modern の bootstrap は既ロード時に非ゼロ終了して
  # set -e で落ちるため単純置換できない）。手動停止の案内は bootout（modern）で統一済み（README/BACKUP.md）。
  launchctl unload "$DEST" 2>/dev/null || true
  launchctl load "$DEST"
  echo "ロードしました: $DEST"
done

echo "確認: launchctl list | grep com.paddock"
echo "ログ: prefetch=/tmp/paddock-prefetch/logs/prefetch.log / keep-awake=/tmp/paddock-keep-awake/logs/keep-awake.log"
echo "      backup-db / backup-staleness=$HOME/Library/Logs/paddock-backup.log（集約ログ。backup-db は毎日 23:30、staleness は毎時 + 起動時）"
echo "      backup-db 詳細は deployments/db/BACKUP.md"
echo "注意: 当日カード（post_time 入り）は朝の paddock-fetch-card で投入済みであること。"
echo "      release バイナリ未ビルドなら: cargo build --release --bin paddock-fetch-card"
echo "取りこぼし確認: python3 scripts/predict-check/snapshot_coverage.py --date <YYYY-MM-DD>"
