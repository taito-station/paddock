#!/usr/bin/env python3
"""ブラインド予想(preds.json)と結果(results.json)を突き合わせ、精度指標を出力する.

指標: 本命的中(単勝)/本命複勝/勝ち馬の model TopK 包含/Brier、芝ダ・距離別、
市場1番人気との比較、本命単勝フラット回収率（win_odds.csv 指定時）。

使い方:
    python3 answer_check.py preds.json results.json [win_odds.csv]

win_odds.csv（任意, ROI と人気比較用）の列: race_id, combination_key(=単勝の馬番), popularity, odds
    sqlite3 -noheader -csv data/paddock.db \
      "SELECT race_id, combination_key, popularity, odds FROM race_odds
       WHERE bet_type='win' AND date='YYYY-MM-DD' ORDER BY race_id, popularity;" > win_odds.csv
"""
import sys
import json
import csv
from collections import defaultdict

if len(sys.argv) < 3:
    print(__doc__, file=sys.stderr)
    sys.exit(1)

with open(sys.argv[1], encoding="utf-8") as f:
    preds = json.load(f)
with open(sys.argv[2], encoding="utf-8") as f:
    results = json.load(f)
odds_csv = sys.argv[3] if len(sys.argv) > 3 else None

SLUG2JP = {"tokyo": "東京", "hanshin": "阪神", "kyoto": "京都", "nakayama": "中山",
           "chukyo": "中京", "sapporo": "札幌", "hakodate": "函館", "fukushima": "福島",
           "niigata": "新潟", "kokura": "小倉"}

# results を (venue_jp, race_num) で索く
res_idx = {(r.get("venue_jp", r.get("venue")), r["race_num"]): r for r in results}

# win_odds.csv → fav[(venue,race_num)] と odds[(venue,race_num)][horse_num]
fav = {}
win_odds = defaultdict(dict)
if odds_csv:
    with open(odds_csv, encoding="utf-8") as f:
        for row in csv.reader(f):
            if len(row) < 4:
                continue
            # 馬番・オッズの不正値（ヘッダ混入等）は行ごとスキップ（popularity と同じ握り方針）。
            try:
                rid, num, pop, od = row[0], int(row[1]), row[2].strip(), float(row[3])
                parts = rid.split("-")  # YYYY-round-slug-day-RRR
                slug = parts[2]
                rn = int(parts[4].replace("R", ""))
            except (ValueError, IndexError):
                continue
            key = (SLUG2JP.get(slug, slug), rn)
            win_odds[key][num] = od
            # popularity は文字列。"1"/"1.0"/空白付きを吸収して 1 番人気を判定。
            # ORDER BY popularity 前提なので最初の 1 番人気のみ採用（後勝ち上書きを避ける）。
            try:
                popi = int(float(pop))
            except ValueError:
                popi = None
            if popi == 1 and key not in fav:
                fav[key] = num


def finish_map(r):
    return {x["horse_num"]: x["rank"] for x in r["rows"] if x["rank"]}


def winner(r):
    for x in r["rows"]:
        if x["rank"] == 1:
            return x["horse_num"]
    return None


n = 0
hon_win = hon_top3 = 0
recall = {1: 0, 2: 0, 3: 0, 4: 0, 5: 0}
brier = 0.0
nh = 0
agg = defaultdict(lambda: [0, 0, 0])  # 区分 -> [n, 本命勝, 本命複勝]
fav_n = fav_win = diff_n = diff_win = 0
bet = pay = 0
log = []

unmatched = []
for p in preds:
    key = (p["venue"], p["race_num"])
    r = res_idx.get(key)
    if not r:
        # venue 表記差・race_num ズレで一部だけ脱落するケースを可視化する（全脱落は後段で停止）。
        unmatched.append(f"{p['venue']}{p['race_num']}R")
        continue
    fmap = finish_map(r)
    # winner() は最初の rank==1 のみ返す（同着1着は稀。recall も単一勝ち馬前提）。
    w = winner(r)
    horses = sorted(p["horses"], key=lambda h: -h["win"])
    ranks = [h["num"] for h in horses]
    hon = horses[0]["num"]
    hon_pos = fmap.get(hon)
    n += 1
    if hon_pos == 1:
        hon_win += 1
    if hon_pos in (1, 2, 3):
        hon_top3 += 1
    for k in recall:
        if w in ranks[:k]:
            recall[k] += 1
    for h in p["horses"]:
        y = 1.0 if fmap.get(h["num"]) == 1 else 0.0
        brier += (h["win"] / 100.0 - y) ** 2
        nh += 1
    surf = p["surface"]
    band = "短" if p["distance"] <= 1400 else "中" if p["distance"] <= 1900 else "長"
    for kk in (surf, f"{surf}{band}"):
        agg[kk][0] += 1
        agg[kk][1] += 1 if hon_pos == 1 else 0
        agg[kk][2] += 1 if hon_pos in (1, 2, 3) else 0
    if key in fav:
        fav_n += 1
        if fmap.get(fav[key]) == 1:
            fav_win += 1
        if fav[key] != hon:
            diff_n += 1
            if hon_pos == 1:
                diff_win += 1
    # 本命オッズが取れたレースのみ ROI 母数に含める（欠損/0.0 を「外れ」と混同しない）。
    o = win_odds.get(key, {}).get(hon)
    if o is not None and o > 0:
        bet += 100
        if hon_pos == 1:
            pay += int(o * 100)
    t3 = [x["horse_num"] for x in sorted([y for y in r["rows"] if y["rank"]], key=lambda y: y["rank"])[:3]]
    mark = "◎" if hon_pos == 1 else ("○" if hon_pos in (2, 3) else "×")
    wr = ranks.index(w) + 1 if w in ranks else None
    log.append((f"{p['venue']}{p['race_num']}R", f"{surf}{p['distance']}", hon,
                hon_pos or "圏外", mark, w, wr or "-", t3, ranks[:4]))

if n == 0:
    print("preds と results が1件もマッチしませんでした（venue 表記/race_num を確認）", file=sys.stderr)
    sys.exit(1)
if unmatched:
    print(f"[warn] results に無く突合できなかった予想 {len(unmatched)}件: {', '.join(unmatched)}",
          file=sys.stderr)

print(f"対象レース: {n}")
print(f"本命的中(単勝): {hon_win}/{n} = {hon_win/n*100:.1f}%")
# 「3着内率」: JRA 複勝は 7 頭以下だと 2 着までのため、厳密な複勝的中とは別物（一律 3 着内で計上）。
print(f"本命3着内率: {hon_top3}/{n} = {hon_top3/n*100:.1f}%")
for k in (1, 2, 3, 4, 5):
    print(f"勝ち馬 model Top{k} 内: {recall[k]}/{n} = {recall[k]/n*100:.1f}%")
if nh:
    print(f"Brier(単勝, 馬あたり平均): {brier/nh:.4f}")
if fav_n:
    print(f"\n市場1番人気 勝率: {fav_win}/{fav_n} = {fav_win/fav_n*100:.1f}%")
    print(f"本命≠1番人気: {diff_n}R（うちモデル本命勝ち {diff_win}）")
if bet:
    print(f"本命単勝フラット回収率: {pay/bet*100:.1f}%（投資¥{bet}→払戻¥{pay}）")

print("\n=== 芝/ダ・距離別 本命成績 ===")
for k in sorted(agg):
    nn, ww, tt = agg[k]
    print(f"{k:<8}n={nn:>2} 本命勝 {ww/nn*100:>3.0f}% 複勝 {tt/nn*100:>3.0f}%")

print("\n=== レース別 ===")
for race, cond, hon, pos, mark, w, wr, t3, mt in log:
    print(f"{race:<8}{cond:<10}本命{hon:>3}→{str(pos):>3}着{mark} 勝馬{w:>3}(model{wr}位) 実3着内{t3} model上位{mt}")
