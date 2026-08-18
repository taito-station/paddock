#!/usr/bin/env bash
# 締切前 prefetch（#237）・keep-awake（#264）・snapshot-coverage（#493）の launchd
# エージェントを停止・除去する（いずれも開催日限定＝朝 install・夜 uninstall の運用）。
# backup-db（#265）・backup-staleness（#490）・verify-backup-restore（#474）・
# purge-snapshots（#492）は install.sh で一括配置するが、ここでは意図的に外さない（非対称）。
# backup-staleness は backup-db を見張る監視で backup-db と対になって常駐する。
# purge-snapshots は snapshot retention を開催日に依らず日次で回す必要があるため常駐にする
# （夜の uninstall で外すと肥大が再開する）。
# prefetch/keep-awake は開催日限定運用（朝 install・夜 uninstall）だが、backup-db は常駐で
# 毎日 23:30 に発火するため。夜の uninstall で backup-db まで外すと、新データを ingest した
# 開催日ほど当夜のバックアップが飛ぶ。常駐エージェントを止めたいときは手動で
# `launchctl bootout gui/$UID/com.paddock.backup-db && rm ~/Library/LaunchAgents/com.paddock.backup-db.plist`
# のように bootout + rm する（BACKUP.md のアンインストール手順と同一。#416）。
set -euo pipefail

LABELS=(com.paddock.prefetch-odds com.paddock.keep-awake com.paddock.snapshot-coverage)
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

# lock の片付けは **trap で到達性から切り離す**。直列に置くと、どこかで set -euo pipefail に
# 引っかかった時点で削除されず「最後まで走ったように見えて走っていない」状態になる
# （#636 の実害がこの形だった）。
#
# ただし **無条件に消すと害がある**——kill に失敗した経路で lock まで消すと、生きている
# caffeinate の pid 記録が失われて以後どの実行からも止められなくなる（keep-awake #264 が
# 避けたい状態）。そこで trap は先に張り、**削除してよいと確定したときだけ**フラグを立てる。
remove_lock=0
trap '[ "${remove_lock}" -eq 1 ] && rm -rf "${LOCK_DIR}" 2>/dev/null; true' EXIT

pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || echo '')"
if [ -z "$pid" ]; then
  # 記録が無い＝止めるべき caffeinate を見失っていないので、残骸を片付けてよい。
  remove_lock=1
elif kill -0 "$pid" 2>/dev/null \
   && ps -p "$pid" -o comm= 2>/dev/null | grep -q 'caffeinate'; then
  if kill "$pid" 2>/dev/null; then
    echo "keep-awake の caffeinate を停止しました（pid ${pid}）"
    remove_lock=1
  else
    echo "⚠ caffeinate (pid ${pid}) を停止できませんでした。lock は残します" >&2
  fi
else
  # pid はあるが生きていない / caffeinate ではない＝stale lock。片付けてよい。
  remove_lock=1
fi
