#!/usr/bin/env bash
# utoipa-swagger-ui の vendored feature が外れていないことを検査する（ADR 0082）。
#
# **外れると build script は無警告でダウンロード分岐へ戻る。** しかも GitHub ランナーには curl が
# あるので、required の ci ジョブは黙って外部取得を再開し、落ちるのは非必須の docker-build だけ
# （`curl: command not found` という原因の分かりにくいエラーで）。つまり退行が静かに起き、
# ADR 0082 が消した障害がそのまま戻る。
#
# 退行経路は 2 つある。(1) リファクタで features から "vendored" が落ちる、(2) dependabot が
# utoipa-swagger-ui のメジャーを上げて feature 名が変わる。
#
# 「人手の規律に委ねない」（ADR 0073）に合わせ、Dockerfile のコメントではなく機械で固定する。
# **この検査自体の回帰テストは scripts/test-check-vendored-swagger.sh**（リポジトリは常に合格側
# なので、検査が壊れても本番データが正常だと無言で緑になる。ADR 0073 と同じ理由でテストを持つ）。
set -euo pipefail

if ! root="$(git rev-parse --show-toplevel 2>/dev/null)"; then
    echo "git リポジトリ外では実行できない（Cargo.toml / Cargo.lock の解決にルートが必要）" >&2
    exit 1
fi
cd "$root"

fail=0

# (1) 主たる判定は Cargo.lock。optional な依存は feature で活性化されない限りロックに載らないので、
#     utoipa-swagger-ui-vendored の在否がそのまま「feature が効いているか」を表す。書式にも依存せず、
#     ci / clippy が --locked で走るためロックの鮮度も担保される。
#     **[[package]] セクション内に限定する**（[[patch.unused]] 等の name 行に誤一致しないため）。
if ! awk '
    /^\[\[package\]\]/ { in_pkg = 1; next }
    /^\[/              { in_pkg = 0 }
    in_pkg && $0 == "name = \"utoipa-swagger-ui-vendored\"" { found = 1 }
    END { exit !found }
' Cargo.lock; then
    echo "✗ Cargo.lock: utoipa-swagger-ui-vendored が解決されていない（feature が効いていない）" >&2
    fail=1
fi

# (2) 宣言側も見る（ロックだけだと「宣言を消したがロックを再生成していない」状態を見逃す）。
#     宣言の開始行から最初の `}` までを切り出し、**行末コメントを落としてから features 配列の中だけ**を
#     見る。単一行の grep にすると (a) features を複数行へ整形しただけで落ち（偽陽性）、
#     (b) 行末コメントに "vendored" と書いてあるだけで通る（偽陰性）。どちらも実際に踏んだ。
decl=$(awk '
    /^utoipa-swagger-ui[[:space:]]*=/ { found = 1 }
    found { sub(/#.*/, ""); print; if (/\}/) exit }
' Cargo.toml)

if [ -z "$decl" ]; then
    echo "✗ Cargo.toml: utoipa-swagger-ui の依存宣言が見つからない（この検査の前提が崩れている）" >&2
    fail=1
else
    features=$(printf '%s ' "$decl" | tr '\n' ' ' \
        | sed -n 's/.*features[[:space:]]*=[[:space:]]*\[\([^]]*\)\].*/\1/p')
    if ! printf '%s\n' "$features" | grep -q '"vendored"'; then
        echo "✗ Cargo.toml: utoipa-swagger-ui の features に \"vendored\" が無い" >&2
        fail=1
    fi
fi

if [ "$fail" -ne 0 ]; then
    echo "  ビルド時に Swagger UI を外部から取得する状態へ戻っている（ADR 0082 を参照）。" >&2
    echo "  上流のメジャー更新で feature 名が変わった場合は、新しい指定へ追従させてから" >&2
    echo "  この検査も直す。" >&2
    exit 1
fi

echo "✓ Swagger UI は vendored（ビルド時の外部取得なし・ADR 0082）"
