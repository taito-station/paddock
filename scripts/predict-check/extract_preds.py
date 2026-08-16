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

from pred_header import HEADER, NoHeaderFound, ensure_header_found

if len(sys.argv) < 2:
    print(__doc__, file=sys.stderr)
    sys.exit(1)

with open(sys.argv[1], encoding="utf-8") as f:
    lines = f.read().splitlines()
races = []
cur = None
hdr = re.compile("^" + HEADER)
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

# 確率テーブルらしい入力なのに 1 レースも取れないのは、入力違いか見出し形式の変化（#587）。
# ここで落とさないと空配列を吐いて exit 0 し、下流の回収率検証が「0 件で成功」として静かに回る。
# 開催の無い日（「この日の開催はありません: …」の 1 行）は馬行が無いので通常どおり [] を返す。
# 判定は split_by_header と同じ ensure_header_found（このスクリプトは行走査なので split は使わない）。
try:
    ensure_header_found("\n".join(lines), len(races), sys.argv[1])
except NoHeaderFound as e:
    sys.exit(str(e))

json.dump(races, sys.stdout, ensure_ascii=False)
print(f"# {len(races)} races", file=sys.stderr)
