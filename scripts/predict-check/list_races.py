#!/usr/bin/env python3
"""指定日・指定場の JRA race_id を列挙する。

使い方:
    python3 list_races.py YYYYMMDD [venue_code ...]
例:
    python3 list_races.py 20260613 05 09      # 東京・阪神のみ
    python3 list_races.py 20260613            # 全場

stdout の 1 列目（race_id）を fetch-card のループや predict 対象の確認に使う。
"""
import sys
from nk import list_race_ids, parse_race_id

if len(sys.argv) < 2:
    print(__doc__)
    sys.exit(1)

date = sys.argv[1]
venues = sys.argv[2:] or None
ids = list_race_ids(date, venues)
for rid in ids:
    p = parse_race_id(rid)
    print(f"{rid}\t{p['venue_jp']}{p['round']}回{p['day']}日 {p['race_num']}R")
print(f"# {len(ids)} races", file=sys.stderr)
