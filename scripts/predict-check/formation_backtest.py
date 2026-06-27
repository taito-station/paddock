#!/usr/bin/env python3
"""上位近接時の2軸フォーメーション バックテスト（#241）。

確定買い方（¥5,000・ワイド/馬連/3連複を model 確率で重み付け配分）を複数開催日の全レースに
機械適用し、上位2頭（◎=最大勝率, ○=2番手）が近接した局面で「◎1頭軸ながし」を
「2軸フォーメーション」に差し替えた時の回収率・的中率・分散を比較する。

発動条件: prob(○) >= θ × prob(◎)（θ を掃引、混戦判定の 0.70 より厳しめの 0.80/0.85/0.90）。
発動レースに限り baseline を以下の variant に差し替える（非発動レースは全 variant 共通 = baseline）:

  - baseline : 全券種 ◎単軸ながし（現行ルール、対照）
  - union2   : 各券種を ◎脚/○脚に予算折半（OR 意味論）。◎が飛んでも ○×穴で拾う。
               ワイド/馬連は ◎-相手 ∪ ○-相手、3連複は ◎軸ながし ∪ ○軸ながし。
               重複（◎○両方を含む目）は ◎脚側に寄せて二重計上を避ける。
  - pair2    : 全券種で ◎○両方必須（AND 意味論）。ワイド/馬連は ◎-○ 1点に集中、
               3連複は ◎○2頭軸ながし（◎-○-相手）。堅いが両崩れで全滅。

回収率の分母は「実際に賭けた額」（賭けられない予算は計上しない）。組成不能で予算が余る
variant を不当に不利化しないため（#180 konsen_backtest と同方針）。

分散は「1レース ROI（払戻/賭金 ×100）」の標準偏差で測る（発動部分集合）。

入力（既定 /tmp、predict-check ハーネスの中間生成物。gen_win_backtest_data.sh で生成）:
  --races       TSV: date, paddock_id, venue_jp, round, day, race_num, netkeiba_id
  --winodds     TSV: paddock_id, umaban, popularity, odds
  --pred-dir    dir: paddock-predict の出力 bt_pred_<date>.txt（確率表）
  --results-dir dir: netkeiba result.html を res_<netkeiba>.html で保存したもの

使い方:
  python3 formation_backtest.py --races /tmp/ft241/bt_races.tsv \
      --winodds /tmp/ft241/bt_winodds.tsv --pred-dir /tmp/ft241 \
      --results-dir /tmp/ft241 --theta-grid 0.80,0.85,0.90
"""
import argparse
import re
import statistics
from itertools import combinations
from pathlib import Path


def largest_remainder(weights, units, minu=1):
    """重み比で units 口を整数配分（各目に最低 minu 口）。konsen_backtest と同一実装。"""
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
        # 口数が頭数に満たない縮退ケース。重みの高い目から 1 口ずつ。units==0 なら全 0。
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
    """netkeiba result.html -> (top3 umaban list, payouts dict)。konsen_backtest と同一。"""
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
        if len(nums) != size * len(yens):
            continue
        for k in range(len(yens)):
            combo = frozenset(nums[k * size:(k + 1) * size])
            if len(combo) == size:
                pay[key][combo] = yens[k]
    return top3, pay


# --- 買い目組成 ---------------------------------------------------------------
# _nagashi_* は [(combo_frozenset, 相手キー), ...] を返す。weight は呼び出し側が確率から算出し
# (combo, weight) の列にして _settle_legs に渡す。


def _nagashi_pairs(axis, partners):
    """軸 axis × partners の 2頭組（ワイド/馬連用）。weight = prob[相手]。"""
    return [(frozenset({axis, p}), p) for p in partners]


def _nagashi_trios(axis, partners):
    """軸 axis × partners から 2 頭選ぶ 3頭組（3連複◎軸ながし用）。weight = prob[a]*prob[b]。"""
    return [(frozenset({axis, a, b}), (a, b)) for a, b in combinations(partners, 2)]


def _settle_legs(legs_weights, budget, pay_table):
    """[(combo, weight)] に budget(円) を重み配分し (払戻, 賭金) を返す。"""
    if not legs_weights or budget < 100:
        return 0, 0
    combos = [c for c, _ in legs_weights]
    weights = [w for _, w in legs_weights]
    units = largest_remainder(weights, budget // 100)
    ret = stake = 0
    for combo, u in zip(combos, units):
        stake += u * 100
        ret += u * 100 * pay_table.get(combo, 0) // 100
    return ret, stake


def settle(probs, top3, pay, variant):
    """1レースを variant の買い方で買い、(払戻, 賭金) を返す。

    A=◎(最大勝率), B=○(2番手), rest=残りを勝率降順。
    baseline/union2/pair2 とも券種予算は ¥5,000（ワイド/馬連/3連複）。
    """
    ranked = sorted(probs, key=lambda n: -probs[n])
    A = ranked[0]
    others = ranked[1:]
    if not others:
        return 0, 0
    B = others[0]
    holes = others[1:]  # ○を除いた穴馬（勝率降順）

    ret = stake = 0

    if variant == "baseline":
        # 全券種 ◎単軸ながし（現行ルール）。ワイド相手 top3 / 馬連・3連複相手 top5。
        wide = [(c, probs[p]) for c, p in _nagashi_pairs(A, others[:3])]
        uma = [(c, probs[p]) for c, p in _nagashi_pairs(A, others[:5])]
        trio = [(c, probs[a] * probs[b]) for c, (a, b) in _nagashi_trios(A, others[:5])]
        for legs, bud, bt in ((wide, 1500, "wide"), (uma, 1500, "umaren"), (trio, 2000, "trio")):
            r, s = _settle_legs(legs, bud, pay[bt])
            ret += r
            stake += s
        return ret, stake

    if variant == "union2":
        # ◎脚 ∪ ○脚に予算折半。◎○両方を含む目は ◎脚に寄せ二重計上を防ぐ。
        # ◎脚の相手は [B]+holes、○脚の相手は holes のみ。
        a_pool = [B] + holes
        b_pool = holes
        # ワイド ¥1,500 → 各脚 ¥750（相手 top3）
        for axis, pool, bud in ((A, a_pool, 750), (B, b_pool, 750)):
            legs = [(c, probs[p]) for c, p in _nagashi_pairs(axis, pool[:3])]
            r, s = _settle_legs(legs, bud, pay["wide"])
            ret += r
            stake += s
        # 馬連 ¥1,500 → 各脚 ¥750（相手 top5）
        for axis, pool, bud in ((A, a_pool, 750), (B, b_pool, 750)):
            legs = [(c, probs[p]) for c, p in _nagashi_pairs(axis, pool[:5])]
            r, s = _settle_legs(legs, bud, pay["umaren"])
            ret += r
            stake += s
        # 3連複 ¥2,000 → 各脚 ¥1,000（軸ながし、相手 top5）
        for axis, pool, bud in ((A, a_pool, 1000), (B, b_pool, 1000)):
            legs = [(c, probs[a] * probs[b]) for c, (a, b) in _nagashi_trios(axis, pool[:5])]
            r, s = _settle_legs(legs, bud, pay["trio"])
            ret += r
            stake += s
        return ret, stake

    if variant == "pair2":
        # 全券種 ◎○両方必須（AND）。ワイド/馬連は ◎-○ に集中、3連複は ◎○2頭軸ながし。
        rw, sw = _settle_legs([(frozenset({A, B}), 1.0)], 1500, pay["wide"])
        rm, sm = _settle_legs([(frozenset({A, B}), 1.0)], 1500, pay["umaren"])
        trio = [(frozenset({A, B, p}), probs[p]) for p in holes[:5]]
        rt, st = _settle_legs(trio, 2000, pay["trio"])
        return rw + rm + rt, sw + sm + st

    raise ValueError(f"unknown variant: {variant}")


def is_close(probs, theta):
    """上位2頭が近接（prob(○) >= theta × prob(◎)）なら True。"""
    ranked = sorted(probs.values(), reverse=True)
    if len(ranked) < 2 or ranked[0] <= 0:
        return False
    return ranked[1] >= theta * ranked[0]


def fmt_stats(rows):
    """[(ret, stake)] -> 'ROI% / 的中率% / σ(per-race ROI)' 文字列。"""
    if not rows:
        return f"{'-':>7} {'-':>6} {'-':>7}"
    tot_ret = sum(r for r, _ in rows)
    tot_stake = sum(s for _, s in rows)
    roi = tot_ret / tot_stake * 100 if tot_stake else 0
    hits = sum(1 for r, _ in rows if r > 0)
    hit = hits / len(rows) * 100
    per = [r / s * 100 if s else 0 for r, s in rows]
    sd = statistics.pstdev(per) if len(per) > 1 else 0.0
    return f"{roi:>6.1f}% {hit:>5.0f}% {sd:>7.1f}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--races", default="/tmp/ft241/bt_races.tsv")
    ap.add_argument("--winodds", default="/tmp/ft241/bt_winodds.tsv")
    ap.add_argument("--pred-dir", default="/tmp/ft241")
    ap.add_argument("--results-dir", default="/tmp/ft241")
    ap.add_argument("--theta-grid", default="0.80,0.85,0.90")
    args = ap.parse_args()

    races = parse_races(args.races)
    winodds = parse_winodds(args.winodds)
    preds = {}
    for d in sorted({r["date"] for r in races}):
        p = Path(args.pred_dir) / f"bt_pred_{d}.txt"
        if p.exists():
            preds[d] = parse_pred(p)

    thetas = [float(x) for x in args.theta_grid.split(",")]
    variants = ["baseline", "union2", "pair2"]

    # 各レースを 1 度だけ評価し、(probs, top3, pay) を保持。
    evaluated = []  # (date, venue, rnum, probs, settle結果dict)
    skips = dict(probs=0, odds=0, result=0)
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
        res = {v: settle(probs, top3, pay, v) for v in variants}
        evaluated.append((r["date"], r["venue"], r["rnum"], probs, res))

    used = len(evaluated)
    skipped = sum(skips.values())
    print(
        f"対象レース: {used}（スキップ {skipped}: probs欠落 {skips['probs']} / "
        f"odds欠落 {skips['odds']} / result欠落 {skips['result']}）\n"
    )

    # 全レース（非発動は baseline 固定）での portfolio ROI。発動時のみ variant 差替え。
    print("=== 発動部分集合での比較（baseline vs 2軸） ===")
    print(f"{'theta':>6} {'発動R':>5}  {'variant':<9} {'ROI':>7} {'的中':>6} {'σROI':>7}")
    for theta in thetas:
        subset = [(d, v, rn, res) for d, v, rn, probs, res in evaluated if is_close(probs, theta)]
        n = len(subset)
        for variant in variants:
            rows = [res[variant] for _, _, _, res in subset]
            label = f"{theta:>6.2f} {n:>5}" if variant == "baseline" else f"{'':>6} {'':>5}"
            print(f"{label}  {variant:<9} {fmt_stats(rows)}")
        print()

    # 全体（発動時のみ差替え）の portfolio ROI も併記。day 全レースに張る前提の総回収率。
    print(f"=== 全 {used}R 通し（非発動=baseline, 発動時のみ差替え）の総回収率 ===")
    print(f"{'theta':>6}  {'strategy':<9} {'ROI':>7} {'的中':>6}")
    for theta in thetas:
        for variant in variants:
            rows = []
            for d, v, rn, probs, res in evaluated:
                key = variant if is_close(probs, theta) else "baseline"
                rows.append(res[key])
            tot_ret = sum(r for r, _ in rows)
            tot_stake = sum(s for _, s in rows)
            roi = tot_ret / tot_stake * 100 if tot_stake else 0
            hit = sum(1 for r, _ in rows if r > 0) / len(rows) * 100 if rows else 0
            label = f"{theta:>6.2f}" if variant == "baseline" else f"{'':>6}"
            print(f"{label}  {variant:<9} {roi:>6.1f}% {hit:>5.0f}%")
        print()


if __name__ == "__main__":
    main()
