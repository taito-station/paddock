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
from nk import list_race_ids, parse_race_id, parse_payouts, result_page

if len(sys.argv) < 2:
    print(__doc__, file=sys.stderr)
    sys.exit(1)

date = sys.argv[1]
venues = sys.argv[2:] or None
ids = list_race_ids(date, venues)

out = []
for rid in ids:
    p = parse_race_id(rid)
    # 取得間隔・確定ページのキャッシュは nk.result_page が持つ（#763）。parse_payouts の warn は残す
    # （券種ごとの組合せ件数と配当件数の不一致は金額に直結するので、未完ページの warn と重なっても出す）
    p["payouts"] = parse_payouts(result_page(rid), rid)
    out.append(p)
    win = p["payouts"].get("win", {})
    note = " ".join(f"{k}={v}" for k, v in win.items()) or "（払戻なし）"
    print(f"{p['venue_jp']}{p['race_num']:>2}R 単勝 {note}", file=sys.stderr)

json.dump(out, sys.stdout, ensure_ascii=False)
print(f"# saved {len(out)} races", file=sys.stderr)
