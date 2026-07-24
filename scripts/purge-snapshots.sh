#!/usr/bin/env bash
# race_odds_snapshots の retention を日次で適用する launchd ラッパ（#492）。
#
# race_odds_snapshots は締切前 live オッズを 15 分毎に append する再取得不能資産だが、
# ≈30MB/日・年 ≈11GB ペースで単調増加する。放置すると Colima VM ディスクと dump サイズ
# （backup 時間・off-machine 転送量に直結）が黙って肥大し、気づくのは VM ディスク枯渇か
# backup 遅延時になる。これを防ぐため保持月数 (PADDOCK_PURGE_MONTHS) より古い snapshot を
# 日次で削除する。日次実行は deployments/launchd/com.paddock.purge-snapshots.plist。
#
# 本体は `paddock-analyze purge-snapshots --months <N>`（src/apps/analyze/src/cli.rs の
# PurgeSnapshots）。cutoff = 実行日(UTC) − months で、fetched_at がそれより前の snapshot 行のみ
# 削除する（最新キャッシュ race_odds は消さない）。#218（live オッズで α 再校正）が必要とする
# 直近 3〜6 ヶ月は必ず残す保持月数を既定にする。
#
# 保持月数の既定 = 6 ヶ月:
#   #218 が要する 3〜6 ヶ月の上端。CLI 既定の 12 は安全側の天井だが、6 ヶ月でも #218 の要件を
#   満たしつつ定常ディスクを ≈5.4GB（12 ヶ月の ≈11GB の約半分）に抑える。運用でより長く残したい
#   場合は PADDOCK_PURGE_MONTHS で上書きする（下限ガードは CLI 側が 0 のみ弾く）。
#
# 使い方:
#   scripts/purge-snapshots.sh                       # 既定 6 ヶ月保持で削除
#   PADDOCK_PURGE_MONTHS=12 scripts/purge-snapshots.sh
#   scripts/purge-snapshots.sh --dry-run             # 削除せず該当行数のみ表示
set -euo pipefail

log() { echo "[$(date '+%Y-%m-%dT%H:%M:%S%z')] $*"; }

usage() {
    cat <<'EOF'
purge-snapshots.sh - race_odds_snapshots の retention を日次適用する（#492）

使い方:
  scripts/purge-snapshots.sh              # 既定保持月数で古い snapshot を削除
  scripts/purge-snapshots.sh --dry-run    # 削除せず該当行数のみ表示
  scripts/purge-snapshots.sh -h|--help

環境変数:
  PADDOCK_PURGE_MONTHS   保持月数（これより古い fetched_at の snapshot を削除）。
                         既定: 6（#218 が要する直近 3〜6 ヶ月を確保しつつディスクを抑える）。
EOF
}

DRY_RUN=0
case "${1:-}" in
    -h|--help) usage; exit 0 ;;
    --dry-run) DRY_RUN=1 ;;
    "") ;;
    *) echo "不明な引数: $1" >&2; usage >&2; exit 2 ;;
esac

MONTHS="${PADDOCK_PURGE_MONTHS:-6}"

# MONTHS は正整数のみ許可（0 は CLI 側の下限ガードで弾かれるが、非整数はここで早期に落とす）。
if ! [[ "$MONTHS" =~ ^[1-9][0-9]*$ ]]; then
    echo "PADDOCK_PURGE_MONTHS は正整数である必要がある: $MONTHS" >&2
    exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# analyze バイナリは PADDOCK_DB_URL を .env（dotenvy）から cwd 相対で読むため、REPO_ROOT へ移動して
# 実行する（launchd の cwd はリポ外のため）。release バイナリを使う（debug ビルドでの運用を防ぐ,
# prefetch_odds.sh と同方針）。
cd "$REPO_ROOT"
BIN="$REPO_ROOT/target/release/paddock-analyze"
if [[ ! -x "$BIN" ]]; then
    log "release バイナリが見つかりません: $BIN"
    log "先に: cd $REPO_ROOT && cargo build --release --bin paddock-analyze"
    exit 1
fi

args=(purge-snapshots --months "$MONTHS")
if [[ "$DRY_RUN" -eq 1 ]]; then
    args+=(--dry-run)
    log "purge-snapshots dry-run 開始（保持 ${MONTHS} ヶ月）"
else
    log "purge-snapshots 開始（保持 ${MONTHS} ヶ月）"
fi

if ! "$BIN" "${args[@]}"; then
    log "FAIL: purge-snapshots が異常終了しました（保持 ${MONTHS} ヶ月）"
    exit 1
fi

log "purge-snapshots 完了（保持 ${MONTHS} ヶ月）"
