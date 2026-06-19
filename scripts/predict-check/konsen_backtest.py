#!/usr/bin/env python3
"""混戦判定の閾値バックテスト（#180）。

確定買い方（¥5,000・ワイド/馬連/3連複を model 確率で重み付け配分・混戦は3連複ボックス追加）を、
複数開催日の全レースに機械適用して回収率を集計する。混戦の発動条件を切り替えて比較する:

  - box-off            : 混戦を一切発動しない純◎軸ながし（ボックス寄与の対照）
  - baseline           : ◎の model 勝率の 0.70 倍以上が ◎含め4頭以上なら混戦
  - odds>=X/band>=B    : baseline に加え「◎単勝 >= X 倍 かつ band>=B 頭」でも混戦扱い（B は 2/3 を掃引）

回収率の分母は「実際に賭けた額」（賭けられない予算は計上しない）。混戦時 band<3 だと3連複
ボックスが組成不能で予算が余るため、これを損失計上して特定 variant を不当に不利化しないため。

入力（既定は /tmp、predict-check ハーネスの中間生成物）:
  --races     TSV: date, paddock_id, venue_jp, round, day, race_num, netkeiba_id
  --winodds   TSV: paddock_id, umaban, popularity, odds
  --pred-dir  dir: paddock-predict の出力 bt_pred_<date>.txt（確率表）
  --results-dir dir: netkeiba result.html を res_<netkeiba>.html で保存したもの

使い方:
  python3 konsen_backtest.py --races /tmp/bt_races.tsv --winodds /tmp/bt_winodds.tsv \
      --pred-dir /tmp --results-dir /tmp --odds-grid 3.0,3.5,4.0
"""
import argparse
import re
from itertools import combinations
from pathlib import Path

BUDGET = 5000


def band_of(probs):
    """◎(最大 win_prob)の 0.70 倍以上の馬を勝率降順で返す（◎を含む）。
    混戦の発動判定（頭数）と3連複ボックスの組成（最大5頭）で同じ band を共有する。"""
    axis = max(probs, key=lambda n: probs[n])
    return sorted([n for n in probs if probs[n] >= 0.70 * probs[axis]], key=lambda n: -probs[n])


def largest_remainder(weights, units, minu=1):
    n = len(weights)
    if n == 0:
        return []
    s = sum(weights)
    if s <= 0:
        weights = [1] * n
        s = n
    base = [minu] * n
    rem = units - minu * n
    if rem < 0:
        # 口数が頭数に満たない縮退ケース。重みの高い目から 1 口ずつ配る（先頭順ではなく）。
        order = sorted(range(n), key=lambda i: weights[i], reverse=True)
        out = [0] * n
        for i in range(units):
            out[order[i]] = 1
        return out
    ideal = [rem * w / s for w in weights]
    fl = [int(x) for x in ideal]
    alloc = [base[i] + fl[i] for i in range(n)]
    left = rem - sum(fl)
    order = sorted(range(n), key=lambda i: ideal[i] - fl[i], reverse=True)
    for i in range(left):
        alloc[order[i % n]] += 1
    return alloc


def parse_races(path):
    rows = []
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        c = line.split("\t")
        rows.append(dict(date=c[0], pid=c[1], venue=c[2], rnum=int(c[5]), nk=c[6]))
    return rows


def parse_winodds(path):
    d = {}
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        pid, uma, pop, odds = line.split("\t")
        d.setdefault(pid, {})[int(uma)] = (int(pop), float(odds))
    return d


def parse_pred(path):
    """predict 出力から (venue_jp, race_num) -> {umaban: win_prob} を抽出。"""
    text = Path(path).read_text()
    # 2 つのキャプチャ群 (race_num, venue) を持つため split の stride は 3（num, venue, body）。
    blocks = re.split(r"--- レース (\d+): (\S+) \S+ \d+m ---", text)
    out = {}
    i = 1
    while i + 2 < len(blocks):
        rnum, venue, body = int(blocks[i]), blocks[i + 1], blocks[i + 2]
        probs = {}
        for line in body.splitlines():
            m = re.match(r"\s*(\d+)\s+\S+\s+([\d.]+)%", line)
            if m:
                probs[int(m.group(1))] = float(m.group(2))
        if probs:
            out[(venue, rnum)] = probs
        i += 3
    return out


def parse_result(path):
    """netkeiba result.html -> (top3 umaban list, payouts dict)。"""
    t = Path(path).read_text(encoding="utf-8")
    rows = re.split(r'<tr\b[^>]*class="[^"]*HorseList[^"]*"', t)[1:]
    order = []
    for r in rows:
        rk = re.search(r'class="Rank">(\d+)</div>', r)
        um = re.search(r'class="Num Txt_C">\s*<div>\s*(\d+)\s*</div>', r, re.S)
        if rk and um:
            order.append((int(rk.group(1)), int(um.group(1))))
    order.sort()
    top3 = [u for _, u in order[:3]]
    pay = {"umaren": {}, "wide": {}, "trio": {}}
    for key, cls in [("umaren", "Umaren"), ("wide", "Wide"), ("trio", "Fuku3")]:
        m = re.search(rf'<tr class="{cls}">(.*?)</tr>', t, re.S)
        if not m:
            continue
        combos = re.findall(r'class="Result">(.*?)</td>', m.group(1), re.S)
        pays = re.findall(r'class="Payout">(.*?)</td>', m.group(1), re.S)
        if not combos or not pays:
            continue
        nums = [int(x) for x in re.findall(r"\d+", re.sub(r"<[^>]+>", " ", combos[0]))]
        yens = [int(x.replace(",", "")) for x in re.findall(r"([\d,]+)円", re.sub(r"<[^>]+>", " ", pays[0]))]
        size = 3 if key == "trio" else 2
        # 馬番列と払戻列の整合チェック。数字数が size×払戻数と合わなければ（注釈混入・構造変化等）、
        # 誤対応で払戻を捏造するより当該券種を空にして取りこぼし側へ倒す。
        if len(nums) != size * len(yens):
            continue
        for k in range(len(yens)):
            combo = frozenset(nums[k * size:(k + 1) * size])
            if len(combo) == size:
                pay[key][combo] = yens[k]
    return top3, pay


def settle_race(probs, winodds, top3, pay, konsen):
    """1レースを確定買い方で買い、(払戻, 実際に賭けた金額) を返す。konsen=True なら3連複ボックス追加。

    実消化額（actual stake）を別途返すのは、混戦時に band<3 で3連複ボックスが組成不能だと
    その券種予算が 1 円も賭けられないため。回収率の分母は「実際に賭けた額」を使い、賭けられない
    予算を損失計上して特定 variant だけを不当にペナルティすることを避ける（#180 レビュー指摘）。"""
    axis = max(probs, key=lambda n: probs[n])
    parts = sorted([n for n in probs if n != axis], key=lambda n: -probs[n])[:5]
    ret = 0
    stake = 0
    if konsen:
        # ボックスは band（最大5頭）の3連複。band<3 なら combos が空＝この券種は賭けられない。
        box = band_of(probs)[:5]
        combos = list(combinations(box, 3))
        bu = largest_remainder([probs[a] * probs[b] * probs[c] for a, b, c in combos], 1500 // 100)
        for (a, b, c), u in zip(combos, bu):
            stake += u * 100
            ret += u * 100 * pay["trio"].get(frozenset({a, b, c}), 0) // 100
        bw, bm, bf = 1000, 1000, 1500
    else:
        bw, bm, bf = 1500, 1500, 2000
    wp = parts[:3]
    for n, u in zip(wp, largest_remainder([probs[n] for n in wp], bw // 100)):
        stake += u * 100
        ret += u * 100 * pay["wide"].get(frozenset({axis, n}), 0) // 100
    for n, u in zip(parts, largest_remainder([probs[n] for n in parts], bm // 100)):
        stake += u * 100
        ret += u * 100 * pay["umaren"].get(frozenset({axis, n}), 0) // 100
    pairs = list(combinations(parts, 2))
    fu = largest_remainder([probs[a] * probs[b] for a, b in pairs], bf // 100)
    # 3連複ながし（◎軸）はボックスと同一の目を重複購入しうるが、実購入として自然なので許容する。
    for (a, b), u in zip(pairs, fu):
        stake += u * 100
        ret += u * 100 * pay["trio"].get(frozenset({axis, a, b}), 0) // 100
    return ret, stake


def is_konsen(probs, axis_odds, odds_thresh, odds_band_min=3):
    band = band_of(probs)
    if len(band) >= 4:
        return True
    if (
        odds_thresh is not None
        and axis_odds is not None
        and axis_odds >= odds_thresh
        and len(band) >= odds_band_min
    ):
        return True
    return False


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--races", default="/tmp/bt_races.tsv")
    ap.add_argument("--winodds", default="/tmp/bt_winodds.tsv")
    ap.add_argument("--pred-dir", default="/tmp")
    ap.add_argument("--results-dir", default="/tmp")
    ap.add_argument("--odds-grid", default="3.0,3.5,4.0")
    args = ap.parse_args()

    races = parse_races(args.races)
    winodds = parse_winodds(args.winodds)
    preds = {}
    for d in sorted({r["date"] for r in races}):
        p = Path(args.pred_dir) / f"bt_pred_{d}.txt"
        if p.exists():
            preds[d] = parse_pred(p)

    # variant = (name, odds_thresh, odds_band_min)。
    #   "box-off" は混戦を一切発動しない（純◎軸ながし）= ボックスの寄与そのものを測る対照。
    #   baseline は band>=4 のみ。
    variants = [("box-off", "OFF", None), ("baseline", None, None)]
    for x in args.odds_grid.split(","):
        for bm in (2, 3):
            variants.append((f"odds>={x}/band>={bm}", float(x), bm))
    agg = {name: dict(stake=0, ret=0, konsen=0, konsen_hit=0) for name, *_ in variants}
    used = 0
    konsen_detail = {name: [] for name, *_ in variants}

    skipped = 0
    for r in races:
        probs = preds.get(r["date"], {}).get((r["venue"], r["rnum"]))
        wo = winodds.get(r["pid"])
        resf = Path(args.results_dir) / f"res_{r['nk']}.html"
        if not probs or not wo or not resf.exists():
            skipped += 1
            continue
        top3, pay = parse_result(resf)
        if len(top3) < 3:
            skipped += 1
            continue
        axis = max(probs, key=lambda n: probs[n])
        axis_odds = wo.get(axis, (None, None))[1]
        used += 1
        for name, thr, bm in variants:
            if thr == "OFF":
                k = False
            elif bm:
                k = is_konsen(probs, axis_odds, thr, bm)
            else:
                k = is_konsen(probs, axis_odds, thr)
            ret, stake = settle_race(probs, wo, top3, pay, k)
            # 回収率の分母は実際に賭けた額（賭けられない予算は計上しない）。
            agg[name]["stake"] += stake
            agg[name]["ret"] += ret
            if k:
                agg[name]["konsen"] += 1
                if ret > 0:
                    agg[name]["konsen_hit"] += 1
                konsen_detail[name].append((r["date"], r["venue"], r["rnum"], axis_odds, ret))

    print(f"対象レース: {used}（データ欠落でスキップ: {skipped}）\n")
    print(f"{'variant':<18} {'回収率':>8} {'損益':>10} {'混戦数':>6} {'混戦的中':>7}")
    for name, *_ in variants:
        a = agg[name]
        roi = a["ret"] / a["stake"] * 100 if a["stake"] else 0
        print(f"{name:<18} {roi:>7.1f}% {a['ret']-a['stake']:>+10} {a['konsen']:>6} {a['konsen_hit']:>7}")

    base_k = {(d, v, rn) for d, v, rn, *_ in konsen_detail["baseline"]}
    print("\n-- odds 条件で新規に混戦化したレースの収支（混戦モード時の払戻）--")
    for name, thr, bm in variants:
        if thr is None or thr == "OFF":
            continue
        added = [x for x in konsen_detail[name] if (x[0], x[1], x[2]) not in base_k]
        if not added:
            print(f"{name}: 追加なし")
            continue
        s = "; ".join(f"{d} {v}{rn}R ◎{o}倍→¥{ret}" for d, v, rn, o, ret in added)
        print(f"{name}: 追加 {len(added)}R [{s}]")


if __name__ == "__main__":
    main()
