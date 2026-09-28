#!/usr/bin/env bash
# paddock Postgres コンテナへの exec を実行環境（lima/nerdctl または colima/docker）に依らず
# 行うための共通ライブラリ（#731）。source して使う（実行しない）。
#
# 背景: 開発機の docker ランタイムは colima（docker）だったが、現在の DB は Lima VM 内の
# rootless nerdctl コンテナで動いている（#731）。backup-db.sh / verify-backup-restore.sh が
# `docker exec` / `docker ps` を直に呼んでいたため、colima が Stopped の間ずっと「コンテナが
# 起動していない」という誤診断で失敗していた。今後どちらのランタイムでも動くよう、実行環境の
# 判定と exec をここに集約する。
#
# 使い方:
#   # shellcheck source=scripts/lib/pg-container.sh
#   source "$(dirname "$0")/lib/pg-container.sh"
#   if ! pg_container_require_running "$CONTAINER"; then
#       exit 1
#   fi
#   pg_container_exec "$CONTAINER" pg_dump -U "$PG_USER" -d "$PG_DB" -Fc > "$dump"
#   pg_container_exec -i "$CONTAINER" pg_restore -U "$PG_USER" -d "$SCRATCH_DB" < "$dump"
#
# 環境変数:
#   PADDOCK_PG_RUNTIME   auto|lima|docker（既定 auto）。
#                        auto: limactl があり VM（PADDOCK_LIMA_VM）が Running なら lima。
#                              そうでなく docker があり対象コンテナが `docker ps` に見えれば docker。
#                              どちらも当たらなければ確認結果を列挙して失敗する。
#   PADDOCK_LIMA_VM      lima ランタイム時の VM 名（既定 paddock）。
#
# macOS 既定の /bin/bash 3.2（launchd もこれで起動する）で動くことを前提にする。
# 連想配列(declare -A)・mapfile は使わない（他の scripts/*.sh と同じ制約）。
#
# pg_container_resolve_runtime / pg_container_running / pg_container_require_running が
# 設定するグローバル変数（呼び出し元スクリプトからは読み取り専用として扱うこと）:
#   PG_RUNTIME     lima|docker（解決成功時のみ設定）
#   PG_RUNTIME_VM  lima のときの VM 名（docker のときは空）
#   PG_RUNTIME_FALLBACK  auto 判定で「limactl はあるが VM が Running でない」ため docker を
#                  選んだとき、その理由（1 行）。それ以外は空。呼び出し側は警告・通知に使う
#                  （移行前の旧 DB を黙って退避・検証しないため。#731）
#   PG_PS_ERROR    pg_container_running が ps コマンド自体の失敗で return 2 したときの stderr 要約

# source 時点で初期化する（呼び出し元の環境に同名変数が export されていても、resolve 前の
# exec が暗黙にその値で動かないようにする）。
PG_RUNTIME=""
PG_RUNTIME_VM=""
PG_RUNTIME_FALLBACK=""
PG_PS_ERROR=""

# lima VM の状態（Running / Stopped / 空=未作成・取得失敗）を返す。
_pg_lima_status() {
    limactl list --format '{{.Status}}' "$1" 2>/dev/null || true
}

# 実行環境を判定し、成功時は PG_RUNTIME（+ lima なら PG_RUNTIME_VM）を設定する。
# 失敗時は判定に使った事実（limactl の有無・VM の状態・docker の応答）を列挙して stderr へ出し、
# 非 0 を返す（黙って進めない）。
#
# 引数: $1 = コンテナ名（auto 判定で docker 側の起動確認に使う）
pg_container_resolve_runtime() {
    local container="$1"
    local requested="${PADDOCK_PG_RUNTIME:-auto}"
    local vm="${PADDOCK_LIMA_VM:-paddock}"

    PG_RUNTIME=""
    PG_RUNTIME_VM=""
    PG_RUNTIME_FALLBACK=""

    case "$requested" in
        lima)
            if ! command -v limactl >/dev/null 2>&1; then
                echo "PADDOCK_PG_RUNTIME=lima だが limactl が見つからない（PATH を確認）" >&2
                return 1
            fi
            # 明示指定でも VM の状態は確認する（停止中に「コンテナが起動していない」と誤診断しない）。
            local explicit_status
            explicit_status="$(_pg_lima_status "$vm")"
            if [[ "$explicit_status" != "Running" ]]; then
                echo "PADDOCK_PG_RUNTIME=lima だが VM ${vm} が ${explicit_status:-未作成または状態取得失敗}（limactl start ${vm} で起動）" >&2
                return 1
            fi
            PG_RUNTIME="lima"
            PG_RUNTIME_VM="$vm"
            return 0
            ;;
        docker)
            if ! command -v docker >/dev/null 2>&1; then
                echo "PADDOCK_PG_RUNTIME=docker だが docker が見つからない（PATH を確認）" >&2
                return 1
            fi
            PG_RUNTIME="docker"
            return 0
            ;;
        auto)
            ;;
        *)
            echo "PADDOCK_PG_RUNTIME は auto|lima|docker のいずれかである必要がある: $requested" >&2
            return 1
            ;;
    esac

    # --- auto 判定: limactl の VM が Running なら lima を優先し、そうでなければ docker に
    # フォールバックする。docker 側は対象コンテナが `docker ps` に見えることまで確認する
    # （docker デーモンが上がっているだけでは判定しない）。
    #
    # lima 側は「VM が Running」で確定し、コンテナが見えなくても docker へは流さない（非対称は意図的）。
    # docker 側（colima 等）には移行前の旧 DB が残っていることがあり、見えた方へ黙って流れると
    # 別インスタンスの DB を退避・検証してしまうため、fail-closed にしている（#731）。
    local limactl_present=0 lima_status="" docker_present=0

    if command -v limactl >/dev/null 2>&1; then
        limactl_present=1
        lima_status="$(_pg_lima_status "$vm")"
        if [[ "$lima_status" == "Running" ]]; then
            PG_RUNTIME="lima"
            PG_RUNTIME_VM="$vm"
            return 0
        fi
    fi

    if command -v docker >/dev/null 2>&1; then
        docker_present=1
        local docker_names
        # パイプ+grep -q は pipefail 下で SIGPIPE により誤判定しうるため、一旦変数へ受けてから
        # 固定文字列(-F)・完全一致(-x)で照合する（backup-db.sh の既存作法と同じ）。
        if docker_names="$(docker ps --format '{{.Names}}' 2>/dev/null)" \
            && grep -qxF "$container" <<<"$docker_names"; then
            PG_RUNTIME="docker"
            if [[ "$limactl_present" -eq 1 ]]; then
                # shellcheck disable=SC2034  # 呼び出し元（backup-db.sh / verify-backup-restore.sh）が読む
                PG_RUNTIME_FALLBACK="lima VM ${vm} が ${lima_status:-未作成または状態取得失敗} のため docker 側のコンテナ ${container} を使った（移行前の旧 DB の可能性がある）"
            fi
            return 0
        fi
    fi

    {
        echo "コンテナ実行環境を自動判定できなかった（PADDOCK_PG_RUNTIME=auto, container=${container}）:"
        if [[ "$limactl_present" -eq 1 ]]; then
            echo "  - limactl: 見つかった（VM ${vm} の状態: ${lima_status:-不明（未作成/取得失敗）}）"
        else
            echo "  - limactl: 見つからない（PATH を確認）"
        fi
        if [[ "$docker_present" -eq 1 ]]; then
            echo "  - docker: 見つかった（コンテナ ${container} は起動中一覧に無い、または docker 未応答）"
        else
            echo "  - docker: 見つからない（PATH を確認）"
        fi
    } >&2
    return 1
}

# 対象コンテナが起動しているか（PG_RUNTIME が解決済みであること・呼び出し前に
# pg_container_resolve_runtime を実行しておく）。
# 戻り値: 0 = 起動中 / 1 = 起動中一覧に無い / 2 = ps コマンド自体が失敗（PG_PS_ERROR に stderr の要約）
pg_container_running() {
    local container="$1"
    local names rc=0

    PG_PS_ERROR=""
    # stderr も同じ変数に受ける（一時ファイルや外部コマンドを増やさない）。成功時に警告行が
    # 混ざっても、下の完全一致(-x)照合なので誤判定しない。
    case "$PG_RUNTIME" in
        lima)
            names="$(limactl shell "$PG_RUNTIME_VM" -- nerdctl ps --format '{{.Names}}' 2>&1)" || rc=$?
            ;;
        docker)
            names="$(docker ps --format '{{.Names}}' 2>&1)" || rc=$?
            ;;
        *)
            echo "pg_container_running: PG_RUNTIME が未解決（先に pg_container_resolve_runtime を呼ぶこと）" >&2
            return 2
            ;;
    esac
    if [[ "$rc" -ne 0 ]]; then
        names="${names//$'\n'/ }"
        PG_PS_ERROR="rc=${rc}: ${names:0:300}"
        return 2
    fi
    grep -qxF "$container" <<<"$names" || return 1
    return 0
}

# ログ・サイドカー用に、解決済みの実行環境を 1 行で返す。
pg_container_describe() {
    printf 'runtime=%s vm=%s container=%s\n' "${PG_RUNTIME:--}" "${PG_RUNTIME_VM:--}" "$1"
}

# 実行環境の解決＋起動確認をまとめた便利関数。両スクリプトで同じ「コンテナが起動していない」の
# 診断・起動コマンド案内を共有するために置く。
pg_container_require_running() {
    local container="$1"

    pg_container_resolve_runtime "$container" || return 1

    local rc=0
    pg_container_running "$container" || rc=$?
    if [[ "$rc" -eq 2 ]]; then
        echo "コンテナ一覧の取得に失敗（$(pg_container_describe "$container")・${PG_PS_ERROR}）" >&2
        return 1
    fi
    if [[ "$rc" -ne 0 ]]; then
        local hint=""
        case "$PG_RUNTIME" in
            lima) hint="limactl shell $PG_RUNTIME_VM -- nerdctl compose -f deployments/compose.yaml up -d postgres" ;;
            docker) hint="docker compose -f deployments/compose.yaml up -d postgres" ;;
        esac
        echo "コンテナ ${container} が起動していない（$(pg_container_describe "$container")・${hint}）" >&2
        return 1
    fi
    return 0
}

# コンテナ内でコマンドを実行する。stdin を渡す場合は先頭に -i を付ける:
#   pg_container_exec [-i] <container> <cmd...>
# PG_RUNTIME が未解決のまま呼ばれた場合は失敗させる（暗黙に docker へフォールバックしない）。
pg_container_exec() {
    # 空配列 "${arr[@]}" は bash 3.2（macOS 既定・launchd 実行環境）の set -u 下で
    # "unbound variable" になる（bash 4.4 以降は平気だが本番は 3.2 前提）ため、配列ではなく
    # stdin_flag の分岐で -i の有無を組み立てる。
    local stdin_flag=0
    if [[ "${1:-}" == "-i" ]]; then
        stdin_flag=1
        shift
    fi
    local container="$1"
    shift

    case "$PG_RUNTIME" in
        lima)
            if [[ "$stdin_flag" -eq 1 ]]; then
                limactl shell "$PG_RUNTIME_VM" -- nerdctl exec -i "$container" "$@"
            else
                limactl shell "$PG_RUNTIME_VM" -- nerdctl exec "$container" "$@"
            fi
            ;;
        docker)
            if [[ "$stdin_flag" -eq 1 ]]; then
                docker exec -i "$container" "$@"
            else
                docker exec "$container" "$@"
            fi
            ;;
        *)
            echo "pg_container_exec: PG_RUNTIME が未解決（先に pg_container_resolve_runtime を呼ぶこと）" >&2
            return 1
            ;;
    esac
}
