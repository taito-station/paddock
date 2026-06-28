#!/usr/bin/env python3
"""配分方式の実ROI比較（#272 Phase2 の money 検証）。

umaren_backtest の baseline ポートフォリオ（馬連top5 ¥1500 + 三連複top5 ¥2000、wide除外）を土台に、
券種内の配分方式だけを変えて 71R の実ROIを比較する。総賭金は方式に依らず一定（=予算）なので、
差は「どの脚に厚く張るか」だけに由来する apples-to-apples 比較。

変種:
  A 現行ルール   : 重み=勝率proxy(probs[p], probs[a]*probs[b]) + minu=1（脚ごと最低¥100）
  B 新Rust       : 重み=真の的中確率(p_top2_set/p_top3_set) + minu=0（薄い脚は¥0）
  C 真確率+minu1 : 重み基準の効果だけ分離
  D proxy+minu0  : minu(最低¥100)撤廃の効果だけ分離
  E 均等割り     : 旧Rust実挙動（重み無し均等）+ minu=0 相当
"""
import sys
from itertools import combinations
from pathlib import Path

sys.path.insert(0, "scripts/predict-check")
import umaren_backtest as ub  # noqa: E402

RACES = "/tmp/bt252/bt_races.tsv"
PRED_DIR = "/tmp/bt252"
RES_DIR = "/tmp/bt252"
EXOTIC = "/tmp/bt252/bt_exotic_odds.tsv"


def load():
    races = ub.parse_races(RACES)
    exotic = ub.parse_exotic(EXOTIC)
    preds = {}
    for d in sorted({r["date"] for r in races}):
        p = Path(PRED_DIR) / f"bt_pred_{d}.txt"
        if p.exists():
            preds[d] = ub.parse_pred(p)
    ev = []
    for r in sorted(races, key=lambda x: (x["date"], x["venue"], x["rnum"])):
        probs = preds.get(r["date"], {}).get((r["venue"], r["rnum"]))
        ex = exotic.get(r["pid"])
        resf = Path(RES_DIR) / f"res_{r['nk']}.html"
        if not probs or not ex or not ex["quinella"] or not resf.exists():
            continue
        top3, pay = ub.parse_result(resf)
        if len(top3) < 3:
            continue
        ev.append((probs, ex["quinella"], ex["trio"], pay))
    return ev


def portfolio_ret_stake(probs, quin_odds, trio_odds, pay, weight, minu):
    """馬連top5 ¥1500 + 三連複top5 ¥2000 を weight/minu 指定で配分し (ret, stake) を返す。"""
    ranked = sorted(probs, key=lambda n: -probs[n])
    if len(ranked) < 3:
        return 0, 0
    A = ranked[0]
    partners = ranked[1:6]

    um_combos = [frozenset({A, p}) for p in partners]
    tr_pairs = list(combinations(partners, 2))
    tr_combos = [frozenset({A, a, b}) for a, b in tr_pairs]

    if weight == "proxy":
        um_w = [probs[p] for p in partners]
        tr_w = [probs[a] * probs[b] for a, b in tr_pairs]
    elif weight == "true":
        um_w = [ub.p_top2_set(probs, A, p) for p in partners]
        tr_w = [ub.p_top3_set(probs, (A, a, b)) for a, b in tr_pairs]
    else:  # equal
        um_w = [1.0] * len(partners)
        tr_w = [1.0] * len(tr_pairs)

    um_stakes = [u * 100 for u in ub.largest_remainder(um_w, 1500 // 100, minu=minu)]
    tr_stakes = [u * 100 for u in ub.largest_remainder(tr_w, 2000 // 100, minu=minu)]

    ret = stake = 0
    for c, s in zip(um_combos, um_stakes):
        stake += s
        ret += s * pay["umaren"].get(c, 0) // 100
    for c, s in zip(tr_combos, tr_stakes):
        stake += s
        ret += s * pay["trio"].get(c, 0) // 100
    return ret, stake


VARIANTS = [
    ("A 現行(proxy,minu1)", "proxy", 1),
    ("B 新Rust(true,minu0)", "true", 0),
    ("C true,minu1", "true", 1),
    ("D proxy,minu0", "proxy", 0),
    ("E 均等(equal,minu0)", "equal", 0),
]


def main():
    ev = load()
    print(f"対象 {len(ev)}R（全鞍機械買い・ゲート無し・wide除外で配分方式のみ比較）\n")
    print(f"{'variant':<22} {'実ROI':>7} {'的中':>5} {'総賭金':>9} {'総払戻':>9}")
    for label, weight, minu in VARIANTS:
        tot_ret = tot_stake = hits = n = 0
        for probs, quin, trio, pay in ev:
            ret, stake = portfolio_ret_stake(probs, quin, trio, pay, weight, minu)
            if stake <= 0:
                continue
            n += 1
            tot_ret += ret
            tot_stake += stake
            hits += 1 if ret > 0 else 0
        roi = tot_ret / tot_stake * 100 if tot_stake else 0
        hit = hits / n * 100 if n else 0
        print(f"{label:<22} {roi:>6.1f}% {hit:>4.0f}% {tot_stake:>9} {tot_ret:>9}")


if __name__ == "__main__":
    main()
