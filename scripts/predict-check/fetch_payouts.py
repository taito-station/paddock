#!/usr/bin/env python3
"""指定日・指定場の確定払戻（馬連/ワイド/三連複等）を netkeiba から取得し JSON で出力する.

戦略別回収率の評価（strategy_eval.py）用。結果着順は fetch_results.py、確率は
extract_preds.py が出すので、本スクリプトは確定配当だけを担当する。

使い方:
    python3 fetch_payouts.py YYYYMMDD [venue_code ...] > payouts.json
例:
    python3 fetch_payouts.py 20260614 05 09 > payouts.json
"""
import sys
import json
import time
from nk import list_race_ids, parse_race_id, fetch_payouts

if len(sys.argv) < 2:
    print(__doc__, file=sys.stderr)
    sys.exit(1)

date = sys.argv[1]
venues = sys.argv[2:] or None
ids = list_race_ids(date, venues)

out = []
for rid in ids:
    p = parse_race_id(rid)
    p["payouts"] = fetch_payouts(rid)
    out.append(p)
    win = p["payouts"].get("win", {})
    note = " ".join(f"{k}={v}" for k, v in win.items()) or "（払戻なし）"
    print(f"{p['venue_jp']}{p['race_num']:>2}R 単勝 {note}", file=sys.stderr)
    time.sleep(0.8)

json.dump(out, sys.stdout, ensure_ascii=False)
print(f"# saved {len(out)} races", file=sys.stderr)
