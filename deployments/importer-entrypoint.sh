#!/bin/sh
# importer の entrypoint。`fetch` サブコマンドのときは JRA への礼節ペーシング既定
# （-j 1 / --interval 3 / --max-rps 0.3）を補う。ノーペーシングだと IP ブロックされうるため
# （ユーザが明示指定した場合はそれを尊重し、重複付与しない）。
set -e

if [ "$1" = "fetch" ]; then
    extra=""
    case " $* " in *" --max-rps "*) ;;            *) extra="$extra --max-rps 0.3" ;; esac
    case " $* " in *" --interval "*) ;;           *) extra="$extra --interval 3" ;; esac
    case " $* " in *" -j "* | *" --parallel "*) ;; *) extra="$extra -j 1" ;; esac
    # $extra は意図的に未クォート（語分割させてフラグとして渡す）。
    exec paddock-parse-pdf "$@" $extra
fi

exec paddock-parse-pdf "$@"
