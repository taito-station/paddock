#!/usr/bin/env python3
"""指定日・指定場のレース結果を netkeiba から取得し JSON で出力する（答え合わせ用）.

`fetch-results`(本体 CLI) は既存 pdf レースの更新専用で新規日を作れないため、
答え合わせ用にこのスクリプトで結果ページを直接パースする。

使い方:
    python3 fetch_results.py YYYYMMDD [venue_code ...] > results.json
例:
    python3 fetch_results.py 20260613 05 09 > results.json
"""
import sys
import json
import time
from nk import list_race_ids, parse_race_id, fetch_result

if len(sys.argv) < 2:
    print(__doc__, file=sys.stderr)
    sys.exit(1)

date = sys.argv[1]
venues = sys.argv[2:] or None
ids = list_race_ids(date, venues)

out = []
for rid in ids:
    p = parse_race_id(rid)
    rows = fetch_result(rid)
    p["rows"] = rows
    out.append(p)
    fin = sorted([x for x in rows if x["rank"]], key=lambda x: x["rank"])[:3]
    top = " ".join(f"{x['rank']}着:{x['horse_num']}番{x['name']}" for x in fin)
    print(f"{p['venue_jp']}{p['race_num']:>2}R n={len(rows):>2} {top}", file=sys.stderr)
    time.sleep(0.8)

json.dump(out, sys.stdout, ensure_ascii=False)
print(f"# saved {len(out)} races", file=sys.stderr)
