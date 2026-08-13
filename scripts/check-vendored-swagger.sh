#!/usr/bin/env bash
# utoipa-swagger-ui の vendored feature が外れていないことを検査する（ADR 0082）。
#
# **外れると build script は無警告でダウンロード分岐へ戻る。** しかも GitHub ランナーには curl が
# あるので、required の ci ジョブは黙って外部取得を再開し、落ちるのは非必須の docker-build だけ
# （`curl: command not found` という原因の分かりにくいエラーで）。つまり退行が静かに起き、
# ADR 0082 が消した障害がそのまま戻る。
#
# 退行経路は 2 つある。(1) リファクタで features から "vendored" が落ちる、(2) dependabot が
# utoipa-swagger-ui のメジャーを上げて feature 名が変わる。どちらも Cargo.toml / Cargo.lock の
# どちらか片方だけでは捕まらないので両方を見る（features に書いてあっても解決されていなければ
# 効いていない）。
#
# 「人手の規律に委ねない」（ADR 0073）に合わせ、Dockerfile のコメントではなく機械で固定する。
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

fail=0

if ! grep -qE '^utoipa-swagger-ui[[:space:]]*=.*"vendored"' Cargo.toml; then
    echo "✗ Cargo.toml: utoipa-swagger-ui の features に \"vendored\" が無い" >&2
    fail=1
fi

if ! grep -q '^name = "utoipa-swagger-ui-vendored"$' Cargo.lock; then
    echo "✗ Cargo.lock: utoipa-swagger-ui-vendored が解決されていない（feature が効いていない）" >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "  ビルド時に Swagger UI を外部から取得する状態へ戻っている（ADR 0082 を参照）。" >&2
    echo "  上流のメジャー更新で feature 名が変わった場合は、新しい指定へ追従させてから" >&2
    echo "  この検査も直す。" >&2
    exit 1
fi

echo "✓ Swagger UI は vendored（ビルド時の外部取得なし・ADR 0082）"
