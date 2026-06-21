#!/usr/bin/env python3
"""条件付き単勝（単勝EV≥閾値の馬だけ）のバックテスト（#208）。

「model 勝率 × 単勝オッズ ≥ 閾値」を満たす馬の単勝を買い目に追加し、
既存3券種（ワイド/馬連/3連複）のみの baseline と回収率・的中率を比較する。

baseline:        ¥5,000 をワイド¥1,500/馬連¥1,500/3連複¥2,000 で運用。
+win≥X%/add:    baseline + 単勝 ¥500 を上乗せ（1レース最大 ¥5,500）。
+win≥X%/split:  単勝 ¥500 を 3 券種から按分で捻出（1レース合計 ¥5,000 相当）。

入力（konsen_backtest.py と同形式）:
  --races      TSV: date, paddock_id, venue_jp, round, day, race_num, netkeiba_id
  --winodds    TSV: paddock_id, umaban, popularity, odds（単勝）
  --pred-dir   dir: bt_pred_<date>.txt（analyze predict の確率表）
  --results-dir dir: res_<netkeiba_id>.html

使い方:
  python3 win_backtest.py --races /tmp/bt_races.tsv --winodds /tmp/bt_winodds.tsv \\
      --pred-dir /tmp --results-dir /tmp --ev-grid 100,110,120
"""
import argparse
import re
from itertools import combinations
from pathlib import Path

BUDGET = 5000
WIN_BUDGET = 500  # 条件付き単勝の 1 発動あたり予算（発動馬が複数なら均等分配）


# ---------------------------------------------------------------------------
# 共通ユーティリティ
# ---------------------------------------------------------------------------

def largest_remainder(weights, units, minu=1):
    """weights 比で units を整数配分（各 minu 以上）。100円単位最大剰余法。"""
    n = len(weights)
    if n == 0:
        return []
    s = sum(weights)
    if s <= 0:
        weights = [1] * n
        s = n
    rem = units - minu * n
    if rem < 0:
        order = sorted(range(n), key=lambda i: weights[i], reverse=True)
        out = [0] * n
        for i in range(min(units, n)):
            out[order[i]] = 1
        return out
    ideal = [rem * w / s for w in weights]
    fl = [int(x) for x in ideal]
    al = [minu + fl[i] for i in range(n)]
    left = rem - sum(fl)
    frac = sorted(range(n), key=lambda i: ideal[i] - fl[i], reverse=True)
    for i in range(left):
        al[frac[i % n]] += 1
    return al


def band_of(probs):
    ax = max(probs, key=lambda n: probs[n])
    return sorted([n for n in probs if probs[n] >= 0.70 * probs[ax]], key=lambda n: -probs[n])


# ---------------------------------------------------------------------------
# 入力パース
# ---------------------------------------------------------------------------

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
        d.setdefault(pid, {})[int(uma)] = float(odds)
    return d


def parse_pred(path):
    text = Path(path).read_text()
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
    """netkeiba result.html → (top3馬番リスト, payouts dict)。

    payouts = {
        "wide":     {frozenset({a, b}): payout_per_100},
        "umaren":   {frozenset({a, b}): payout_per_100},
        "trio":     {frozenset({a, b, c}): payout_per_100},
        "win":      {winner_uma: payout_per_100},
    }
    """
    t = Path(path).read_text(encoding="utf-8")

    # 着順
    rows = re.split(r'<tr\b[^>]*class="[^"]*HorseList[^"]*"', t)[1:]
    order = []
    for r in rows:
        rk = re.search(r'class="Rank">(\d+)</div>', r)
        um = re.search(r'class="Num Txt_C">\s*<div>\s*(\d+)\s*</div>', r, re.S)
        if rk and um:
            order.append((int(rk.group(1)), int(um.group(1))))
    order.sort()
    top3 = [u for _, u in order[:3]]

    pay = {"umaren": {}, "wide": {}, "trio": {}, "win": {}}

    # ワイド/馬連/3連複 は frozenset キー（konsen_backtest.py 踏襲）
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
        if len(nums) != size * len(yens):
            continue
        for k in range(len(yens)):
            combo = frozenset(nums[k * size:(k + 1) * size])
            if len(combo) == size:
                pay[key][combo] = yens[k]

    # 単勝（Tansho）: nk.py の PAYOUT_TYPE と同じ命名
    m = re.search(r'<tr class="Tansho">(.*?)</tr>', t, re.S)
    if m:
        result_cell = re.search(r'class="Result">(.*?)</td>', m.group(1), re.S)
        payout_cell = re.search(r'class="Payout">(.*?)</td>', m.group(1), re.S)
        if result_cell and payout_cell:
            winner_nums = re.findall(
                r'<div[^>]*>\s*<span[^>]*>\s*(\d+)\s*</span>', result_cell.group(1))
            yens = [int(x.replace(",", ""))
                    for x in re.findall(r"([\d,]+)円", re.sub(r"<[^>]+>", " ", payout_cell.group(1)))]
            if winner_nums and yens:
                pay["win"][int(winner_nums[0])] = yens[0]

    return top3, pay


# ---------------------------------------------------------------------------
# 買い目構築と精算
# ---------------------------------------------------------------------------

def settle_race(probs, winodds, top3, pay, ev_thresh, win_mode):
    """1レースを精算し (払戻, 賭け金) を返す。

    ev_thresh: None=baseline / float=EV 閾値（1.0=100%, 1.1=110%, ...）
    win_mode:  None | "add" | "split"
    """
    axis = max(probs, key=lambda n: probs[n])
    parts = sorted([n for n in probs if n != axis], key=lambda n: -probs[n])[:5]

    # 3 券種予算の決定
    if win_mode == "split":
        bw, bm, bf = 1300, 1300, 1800  # 4400 + win500 ≈ ¥4,900（端数¥100 が出うる）
    else:
        bw, bm, bf = 1500, 1500, 2000  # baseline ¥5,000

    ret = 0
    stake = 0

    # ワイド（軸 vs 上位3相手）
    wp = parts[:3]
    for n, u in zip(wp, largest_remainder([probs[n] for n in wp], bw // 100)):
        stake += u * 100
        ret += u * 100 * pay["wide"].get(frozenset({axis, n}), 0) // 100

    # 馬連（軸 vs 上位5相手）
    for n, u in zip(parts, largest_remainder([probs[n] for n in parts], bm // 100)):
        stake += u * 100
        ret += u * 100 * pay["umaren"].get(frozenset({axis, n}), 0) // 100

    # 3連複ながし（軸 1 頭 vs 上位5相手のペア）
    pairs = list(combinations(parts, 2))
    if pairs:
        fu = largest_remainder([probs[a] * probs[b] for a, b in pairs], bf // 100)
        for (a, b), u in zip(pairs, fu):
            stake += u * 100
            ret += u * 100 * pay["trio"].get(frozenset({axis, a, b}), 0) // 100

    # 条件付き単勝
    win_bets = []
    if ev_thresh is not None and winodds:
        qualifying = [
            (n, probs[n] / 100 * odds)
            for n, odds in winodds.items()
            if n in probs and probs[n] / 100 * odds >= ev_thresh
        ]
        qualifying.sort(key=lambda x: -x[1])  # EV 降順
        qualifying = qualifying[:5]  # 最大5頭（予算過多防止）
        if qualifying:
            per_horse = (WIN_BUDGET // len(qualifying) // 100) * 100
            per_horse = max(per_horse, 0)
            for n, _ in qualifying:
                if per_horse >= 100:
                    win_bets.append((n, per_horse))

    for n, amt in win_bets:
        stake += amt
        ret += amt * pay["win"].get(n, 0) // 100

    return ret, stake, len(win_bets)


# ---------------------------------------------------------------------------
# メイン
# ---------------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--races", default="/tmp/bt_races.tsv")
    ap.add_argument("--winodds", default="/tmp/bt_winodds.tsv")
    ap.add_argument("--pred-dir", default="/tmp")
    ap.add_argument("--results-dir", default="/tmp")
    ap.add_argument("--ev-grid", default="100,110,120",
                    help="単勝 EV 閾値（%%, カンマ区切り）")
    args = ap.parse_args()

    races = parse_races(args.races)
    winodds = parse_winodds(args.winodds)
    preds = {}
    for d in sorted({r["date"] for r in races}):
        p = Path(args.pred_dir) / f"bt_pred_{d}.txt"
        if p.exists():
            preds[d] = parse_pred(p)

    ev_thresholds = [float(x) / 100 for x in args.ev_grid.split(",")]

    # variant: (name, ev_thresh, win_mode)
    variants: list[tuple[str, float | None, str | None]] = [("baseline", None, None)]
    for thr in ev_thresholds:
        pct = int(thr * 100)
        variants.append((f"+win≥{pct}%/add", thr, "add"))
        variants.append((f"+win≥{pct}%/split", thr, "split"))

    agg = {name: dict(stake=0, ret=0, win_fired=0, races=0) for name, *_ in variants}

    skips = dict(probs=0, odds=0, result=0)
    used = 0

    for r in races:
        probs = preds.get(r["date"], {}).get((r["venue"], r["rnum"]))
        wo = winodds.get(r["pid"])
        resf = Path(args.results_dir) / f"res_{r['nk']}.html"
        if not probs:
            skips["probs"] += 1
            continue
        if not wo:
            skips["odds"] += 1
            continue
        if not resf.exists():
            skips["result"] += 1
            continue
        top3, pay = parse_result(resf)
        if len(top3) < 3:
            skips["result"] += 1
            continue
        used += 1

        for name, ev_thresh, win_mode in variants:
            ret, stake, n_win = settle_race(probs, wo, top3, pay, ev_thresh, win_mode)
            agg[name]["stake"] += stake
            agg[name]["ret"] += ret
            agg[name]["races"] += 1
            if n_win:
                agg[name]["win_fired"] += n_win

    skipped = sum(skips.values())
    print(f"評価レース数: {used}（スキップ {skipped}: "
          f"probs欠落 {skips['probs']} / odds欠落 {skips['odds']} / result欠落 {skips['result']}）\n")

    base_roi = 0.0
    print(f"{'variant':<22} {'回収率':>8} {'損益':>10} {'単勝発動':>9}")
    for name, *_ in variants:
        a = agg[name]
        roi = a["ret"] / a["stake"] * 100 if a["stake"] else 0.0
        if name == "baseline":
            base_roi = roi
        diff = roi - base_roi
        sign = f"({diff:+.1f}pt)" if name != "baseline" else ""
        fired = a["win_fired"] if a["win_fired"] else "-"
        print(f"{name:<22} {roi:>7.1f}% {sign:<10} {a['ret']-a['stake']:>+10} {fired!s:>9}")


if __name__ == "__main__":
    main()
