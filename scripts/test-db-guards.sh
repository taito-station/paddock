#!/usr/bin/env bash
# reset-db.sh / seed-db.sh の golden DB 保護ガードの回帰テスト（#406）。
#
# DROP DATABASE を伴う破壊スクリプトのガード判定（ブロック / 通過 / fail-closed / --force）を
# 実 DB を一切触らずに検証する。安全のため配置先ホストは常に到達不能ポート :1（即 connection
# refused）を使う。ガードが正しくブロックすればガードのメッセージで exit 1、通過すれば psql/
# pg_dump 段で失敗する（golden メッセージは出ない）。どちらの経路でも実データは破壊されない。
#
# 使い方: bash scripts/test-db-guards.sh   （全ケース PASS で exit 0）
set -uo pipefail

cd "$(dirname "$0")/.."
export PGCONNECT_TIMEOUT=2
# ロケール非依存で全角混じりメッセージを扱う。呼び出し元の PADDOCK_* 汚染を避ける。
export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8
unset PADDOCK_DB_URL PADDOCK_GOLDEN_DB_URL

pass=0
fail=0

# ガードの発火（ブロック / fail-closed）を示すメッセージ。これが出れば「中断された」と判定する。
GUARD_RE='golden DB|配置先が golden|golden URL から database|database 名を取得できない'

# run_case <説明> <BLOCK|ALLOW> <env割当（空可）> -- <スクリプト引数...>
run_case() {
    local desc="$1" expect="$2"; shift 2
    local envassign=() ; while [[ "$1" != "--" ]]; do envassign+=("$1"); shift; done; shift
    local out rc
    # set -u 下で空配列を展開しても落ちないよう ${arr[@]+...} で囲む（bash 3.2 互換）。
    out="$(env "${envassign[@]+"${envassign[@]}"}" bash "$@" 2>&1)"; rc=$?
    if [[ "$expect" == BLOCK ]]; then
        if [[ "$rc" -eq 1 ]] && grep -qE "$GUARD_RE" <<<"$out"; then
            echo "OK  [BLOCK] $desc"; pass=$((pass + 1))
        else
            echo "NG  [BLOCK] $desc (rc=$rc)"; echo "    out: $out"; fail=$((fail + 1))
        fi
    else # ALLOW: ガードを通過し、golden 系メッセージを出さずに後段で失敗する
        if grep -qE "$GUARD_RE" <<<"$out"; then
            echo "NG  [ALLOW] $desc（誤爆でブロックされた）"; echo "    out: $out"; fail=$((fail + 1))
        else
            echo "OK  [ALLOW] $desc"; pass=$((pass + 1))
        fi
    fi
}

echo "=== reset-db.sh ==="
# 既定 golden = @localhost/paddock。ホスト表記揺れ・完全一致・クエリ付き・別ホスト同名は BLOCK。
run_case "127.0.0.1/paddock（ホスト表記揺れ・バグの核心）" BLOCK -- scripts/reset-db.sh --to "postgres://p:p@127.0.0.1:1/paddock"
run_case "localhost/paddock（完全一致）"                    BLOCK -- scripts/reset-db.sh --to "postgres://p:p@localhost:1/paddock"
run_case "クエリ文字列付き golden"                          BLOCK -- scripts/reset-db.sh --to "postgres://p:p@127.0.0.1:1/paddock?sslmode=disable"
run_case "別ホスト db.internal/paddock（名前ベース）"       BLOCK -- scripts/reset-db.sh --to "postgres://u:p@db.internal:1/paddock"
# worktree DB（別名）と --force は通過。
run_case "worktree DB paddock_wt1"                          ALLOW -- scripts/reset-db.sh --to "postgres://p:p@127.0.0.1:1/paddock_wt1"
run_case "golden 名だが --force"                            ALLOW -- scripts/reset-db.sh --force --to "postgres://p:p@127.0.0.1:1/paddock"
# カスタム golden 名は env 由来（ハードコードでない）。
run_case "custom golden mygolden を別ホスト（BLOCK）"       BLOCK PADDOCK_GOLDEN_DB_URL="postgres://u:p@localhost:5432/mygolden" -- scripts/reset-db.sh --to "postgres://u:p@127.0.0.1:1/mygolden"
run_case "golden=mygolden のとき paddock は許可"           ALLOW PADDOCK_GOLDEN_DB_URL="postgres://u:p@localhost:5432/mygolden" -- scripts/reset-db.sh --to "postgres://u:p@127.0.0.1:1/paddock"
# fail-closed: 不正 golden URL（パス無し / 末尾スラッシュ）は名前ベース保護が効かないため中断。
run_case "fail-closed: パス無し golden URL"                 BLOCK PADDOCK_GOLDEN_DB_URL="postgres://p:p@localhost:5432" -- scripts/reset-db.sh --to "postgres://p:p@127.0.0.1:1/paddock"
run_case "fail-closed: 末尾スラッシュ golden URL"           BLOCK PADDOCK_GOLDEN_DB_URL="postgres://p:p@localhost:5432/" -- scripts/reset-db.sh --to "postgres://p:p@127.0.0.1:1/paddock"
# 不正な配置先 URL（db 名を取れない）も中断。
run_case "パス無し 配置先 URL"                              BLOCK -- scripts/reset-db.sh --to "postgres://p:p@127.0.0.1:1"
# --force は不正 golden 検証もスキップ（保護を意図的に外す）。
run_case "--force は不正 golden でもバイパス"               ALLOW PADDOCK_GOLDEN_DB_URL="postgres://p:p@localhost:5432" -- scripts/reset-db.sh --force --to "postgres://p:p@127.0.0.1:1/paddock"

echo "=== seed-db.sh ==="
run_case "from localhost/paddock, to 127.0.0.1/paddock"    BLOCK -- scripts/seed-db.sh --from "postgres://p:p@localhost:1/paddock" --to "postgres://p:p@127.0.0.1:1/paddock"
run_case "クエリ違いの同一 golden"                          BLOCK -- scripts/seed-db.sh --from "postgres://p:p@localhost:1/paddock" --to "postgres://p:p@localhost:1/paddock?sslmode=require"
run_case "worktree DB paddock_wt1（通過）"                  ALLOW -- scripts/seed-db.sh --from "postgres://p:p@127.0.0.1:1/paddock" --to "postgres://p:p@127.0.0.1:1/paddock_wt1"
run_case "fail-closed: パス無し golden(from) URL"           BLOCK -- scripts/seed-db.sh --from "postgres://p:p@127.0.0.1:1" --to "postgres://p:p@127.0.0.1:1/paddock_wt1"
run_case "fail-closed: 末尾スラッシュ golden(from) URL"     BLOCK -- scripts/seed-db.sh --from "postgres://p:p@127.0.0.1:1/" --to "postgres://p:p@127.0.0.1:1/paddock_wt1"
run_case "パス無し 配置先(to) URL"                          BLOCK -- scripts/seed-db.sh --from "postgres://p:p@127.0.0.1:1/paddock" --to "postgres://p:p@127.0.0.1:1"

echo
echo "=== 合計: PASS=${pass} FAIL=${fail} ==="
[[ "$fail" -eq 0 ]]
