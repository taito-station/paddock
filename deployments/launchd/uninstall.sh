#!/usr/bin/env bash
# 締切前 prefetch（#237）と keep-awake（#264）の launchd エージェントを停止・除去する。
# backup-db（#265）は install.sh で一括配置するが、ここでは意図的に外さない（非対称）。
# prefetch/keep-awake は開催日限定運用（朝 install・夜 uninstall）だが、backup-db は常駐で
# 毎日 23:30 に発火するため。夜の uninstall で backup-db まで外すと、新データを ingest した
# 開催日ほど当夜のバックアップが飛ぶ。backup-db を止めたいときは手動で
# `launchctl unload ~/Library/LaunchAgents/com.paddock.backup-db.plist && rm` する（#416）。
set -euo pipefail

LABELS=(com.paddock.prefetch-odds com.paddock.keep-awake)
for LABEL in "${LABELS[@]}"; do
  DEST="$HOME/Library/LaunchAgents/$LABEL.plist"
  if [ -f "$DEST" ]; then
    launchctl unload "$DEST" 2>/dev/null || true
    rm -f "$DEST"
    echo "除去しました: $DEST"
  else
    echo "未インストール: $DEST は存在しません"
  fi
done

# keep-awake は plist の AbandonProcessGroup=true で caffeinate を PGID から切り離すため、
# unload だけでは背景 caffeinate が最終 post_time まで残りスリープ抑止が居座る。lock に記録した
# 自分の pid を comm 確認のうえ kill して即停止する（無差別 pkill はユーザー自身の caffeinate を
# 巻き込むため使わない）。
LOCK_DIR="/tmp/paddock-keep-awake.lock.d"
pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || echo '')"
if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null \
   && ps -p "$pid" -o comm= 2>/dev/null | grep -q 'caffeinate'; then
  kill "$pid" 2>/dev/null && echo "keep-awake の caffeinate を停止しました（pid $pid）"
fi
rm -rf "$LOCK_DIR" 2>/dev/null || true
