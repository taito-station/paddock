#!/usr/bin/env python3
"""predict(スキップモード)の stdout から各レースの確率テーブルを抽出して JSON 化する.

predict をスキップ入力で流した stdout（確率表＋推奨買い目）を読み、レースごとに
馬番・馬名・勝率・連対率・複勝率を取り出す。推奨買い目はノイズが大きいため使わない。

使い方:
    python3 extract_preds.py predict_out.log > preds.json
"""
import sys
import re
import json

if len(sys.argv) < 2:
    print(__doc__, file=sys.stderr)
    sys.exit(1)

with open(sys.argv[1], encoding="utf-8") as f:
    lines = f.read().splitlines()
races = []
cur = None
hdr = re.compile(r"^--- レース (\d+): (\S+) (\S+) (\d+)m ---")
row = re.compile(r"^\s*(\d+)\s+(\S+)\s+([\d.]+)%\s+([\d.]+)%\s+([\d.]+)%\s*$")
for ln in lines:
    m = hdr.match(ln)
    if m:
        cur = {"race_num": int(m.group(1)), "venue": m.group(2),
               "surface": m.group(3), "distance": int(m.group(4)), "horses": []}
        races.append(cur)
        continue
    if cur is not None:
        r = row.match(ln)
        if r:
            cur["horses"].append({
                "num": int(r.group(1)), "name": r.group(2),
                "win": float(r.group(3)), "place": float(r.group(4)), "show": float(r.group(5))})

json.dump(races, sys.stdout, ensure_ascii=False)
print(f"# {len(races)} races", file=sys.stderr)
