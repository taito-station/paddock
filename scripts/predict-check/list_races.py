#!/usr/bin/env python3
"""指定日・指定場の JRA race_id を列挙する。

使い方:
    python3 list_races.py YYYYMMDD [venue_code ...]
例:
    python3 list_races.py 20260613 05 09      # 東京・阪神のみ
    python3 list_races.py 20260613            # 全場

出力は TAB 区切り 3 列:
    1 列目: netkeiba 12桁 race_id（fetch-card のループ用）
    2 列目: 人間可読ラベル
    3 列目: paddock 内部 race_id（race_odds 等を race_id で引く用。race_odds に date 列は無い）
"""
import sys
from nk import list_race_ids, parse_race_id


def paddock_race_id(p):
    return f"{p['year']}-{p['round']}-{p['venue_slug']}-{p['day']}-{p['race_num']}R"


if len(sys.argv) < 2:
    print(__doc__)
    sys.exit(1)

date = sys.argv[1]
venues = sys.argv[2:] or None
ids = list_race_ids(date, venues)
for rid in ids:
    p = parse_race_id(rid)
    label = f"{p['venue_jp']}{p['round']}回{p['day']}日 {p['race_num']}R"
    print(f"{rid}\t{label}\t{paddock_race_id(p)}")
print(f"# {len(ids)} races", file=sys.stderr)
