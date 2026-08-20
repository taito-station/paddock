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
# lock は UID スコープの固定パス（#643）。**scripts/predict-check/keep_awake.sh と同じ式**で、
# 片方だけ変えると uninstall が caffeinate を止められなくなる（＝抑止が居座る）ので必ず同時に直す。
# `$TMPDIR` を使わないのは、launchd が TMPDIR を設定せず端末と別パスに解決されるため
# （2026-08-19 実測）。理由の詳細は keep_awake.sh 側のコメントが正。
LOCK_DIR="${PADDOCK_KEEP_AWAKE_LOCK_DIR:-/tmp/paddock-keep-awake-$(id -u).lock.d}"
# 移行期は**旧パス（uid 無し）**にも caffeinate が記録されていることがある。回収を keep_awake.sh
# 側だけに入れると、新コードの tick が 1 度も走らないうちに夜の uninstall を叩いたときに旧
# caffeinate を止められず、最終 post_time まで抑止が居座る——QA Q1 が `$TMPDIR` を却下した理由
# （「uninstall が caffeinate を止められない」）と同じ壊れ方を、移行経路で自ら作ることになる。
LEGACY_LOCK_DIR="${PADDOCK_KEEP_AWAKE_LEGACY_LOCK_DIR:-/tmp/paddock-keep-awake.lock.d}"

# lock の片付けは **trap で到達性から切り離す**。直列に置くと、どこかで set -euo pipefail に
# 引っかかった時点で削除されず「最後まで走ったように見えて走っていない」状態になる
# （#636 の実害がこの形だった）。
#
# ただし **無条件に消すと害がある**——kill に失敗した経路で lock まで消すと、生きている
# caffeinate の pid 記録が失われて以後どの実行からも止められなくなる（keep-awake #264 が
# 避けたい状態）。そこで trap は先に張り、**削除してよいと確定したときだけ**フラグを立てる。
remove_lock=0
remove_legacy_lock=0
trap '[ "${remove_lock}" -eq 1 ] && rm -rf "${LOCK_DIR}" 2>/dev/null;
      [ "${remove_legacy_lock}" -eq 1 ] && rm -rf "${LEGACY_LOCK_DIR}" 2>/dev/null; true' EXIT

# lock に記録された caffeinate を comm 確認のうえ停止する。
# 戻り値 0 = lock を消してよい / 1 = 消してはいけない（生きているのに止められなかった）。
stop_recorded_caffeinate() {
  local dir="$1" label="$2" pid
  pid="$(cat "$dir/pid" 2>/dev/null || echo '')"
  if [ -z "$pid" ]; then
    # 記録が無い＝止めるべき caffeinate を見失っていないので、残骸を片付けてよい。
    return 0
  fi
  if kill -0 "$pid" 2>/dev/null \
     && ps -p "$pid" -o comm= 2>/dev/null | grep -q 'caffeinate'; then
    if kill "$pid" 2>/dev/null; then
      echo "keep-awake の caffeinate を停止しました（pid ${pid}${label}）"
      return 0
    fi
    echo "⚠ caffeinate (pid ${pid}${label}) を停止できませんでした。lock は残します" >&2
    return 1
  fi
  # pid はあるが生きていない / caffeinate ではない＝stale lock。片付けてよい。
  return 0
}

if stop_recorded_caffeinate "$LOCK_DIR" ""; then
  remove_lock=1
fi

# 旧パスは symlink / 他ユーザー所有なら触らない（pid の中身がそのまま kill の引数になるため）。
if [ "$LOCK_DIR" != "$LEGACY_LOCK_DIR" ] && [ -d "$LEGACY_LOCK_DIR" ] \
   && [ ! -L "$LEGACY_LOCK_DIR" ] && [ -O "$LEGACY_LOCK_DIR" ]; then
  if stop_recorded_caffeinate "$LEGACY_LOCK_DIR" "・旧 lock パス"; then
    remove_legacy_lock=1
  fi
fi
