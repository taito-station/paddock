#!/bin/sh
# importer の entrypoint。`fetch` のときは JRA への礼節ペーシング既定
# （-j 1 / --interval 3 / --max-rps 0.3）を補う（ノーペーシングだと IP ブロックされうるため）。
# ユーザが明示した値は尊重し重複付与しない。検出はフラグ名の前方一致で、空白形（`--max-rps 1`）・
# `=` 連結形（`--max-rps=1`）・短縮連結形（`-j2`）のいずれも拾う。
set -eu

if [ "${1:-}" = "fetch" ]; then
    shift
    extra=""
    case " $* " in *" --max-rps"*) ;;            *) extra="$extra --max-rps 0.3" ;; esac
    case " $* " in *" --interval"*) ;;           *) extra="$extra --interval 3" ;; esac
    case " $* " in *" -j"* | *" --parallel"*) ;; *) extra="$extra -j 1" ;; esac
    # 既定は fetch 直後（ユーザ引数より前）に挿入。$extra は意図的に未クォート（語分割でフラグ化）。
    exec paddock-parse-pdf fetch $extra "$@"
fi

exec paddock-parse-pdf "$@"
