#!/usr/bin/env python3
"""候補A: 市場の系統誤差（人気-穴バイアス）を既存データで測る（本番非変更・measure-first probe）。

純 resolution 天井（ADR 0058）後に残る edge 候補として「市場 implied 確率自体がオッズ帯で
系統的にズレる（人気-穴バイアス）なら、較正補正で EV が取れるのでは」を検証する。純モデルとは独立。

pure dump（win_odds 列・着順列）の gated レース runner を単勝オッズ帯で層別:
- 較正: takeout 除去後の正規化 implied 確率 vs 実勝率（帯ごと・バイアス検出）。
- ROI:  生オッズで各帯を単勝 blind bet した回収率 = mean(won × odds)。+ の帯があれば実 edge。
確率鏡映は不要（市場側だけ見る）。feature_resolution_diag を load/group/_fp/_implied のみ流用。

使い方:
  python3 market_calibration.py --tsv /tmp/pa/pure.tsv
"""
from __future__ import annotations
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import feature_resolution_diag as D  # noqa: E402

BUCKETS = [(1.0, 1.5), (1.5, 2.0), (2.0, 3.0), (3.0, 5.0), (5.0, 7.0), (7.0, 10.0),
           (10.0, 15.0), (15.0, 20.0), (20.0, 30.0), (30.0, 50.0), (50.0, 1e9)]


def bucket_of(o):
    for lo, hi in BUCKETS:
        if lo <= o < hi:
            return (lo, hi)
    return None


def run(tsv):
    rows = D.load(tsv)
    races = D.group_by_race(rows)
    agg = {b: {"n": 0, "wins": 0, "odds_sum": 0.0, "imp_sum": 0.0, "roi_sum": 0.0} for b in BUCKETS}
    overround = []
    nrace = 0
    for _rid, rr in races.items():
        fp = [D._fp(r) for r in rr]
        y = [1 if p == 1 else 0 for p in fp]
        implied, s = D._implied(rr)
        if not (any(y) and s > 0):  # feature_resolution_diag と同一 gated 母数
            continue
        nrace += 1
        overround.append(s)  # s = Σ(1/odds) = overround（>1 で takeout 込み）
        for i, r in enumerate(rr):
            raw = r[D.COL["win_odds"]]
            if not raw:
                continue
            o = float(raw)
            if o < 1.0:
                continue
            b = bucket_of(o)
            if b is None:
                continue
            a = agg[b]
            a["n"] += 1; a["wins"] += y[i]; a["odds_sum"] += o
            a["imp_sum"] += implied[i]; a["roi_sum"] += y[i] * o

    orr = sum(overround) / len(overround)
    print(f"# gated races={nrace}  平均 overround Σ(1/odds)={orr:.3f}  (→ takeout ≈ {100*(1-1/orr):.1f}%)\n")
    print(f"{'odds帯':>12} {'n':>6} {'実勝率':>7} {'正規implied':>10} {'差(実-imp)':>10} {'平均odds':>8} {'単勝ROI':>8}")
    tot_n = 0
    tot_roi = 0.0
    for b in BUCKETS:
        a = agg[b]
        if a["n"] == 0:
            continue
        wr = a["wins"] / a["n"]; imp = a["imp_sum"] / a["n"]
        mo = a["odds_sum"] / a["n"]; roi = a["roi_sum"] / a["n"]
        tot_n += a["n"]; tot_roi += a["roi_sum"]
        lab = f"{b[0]:g}-{b[1]:g}" if b[1] < 1e9 else f"{b[0]:g}+"
        print(f"{lab:>12} {a['n']:>6} {wr:>7.3f} {imp:>10.3f} {wr-imp:>+10.3f} {mo:>8.1f} {roi:>8.3f}")
    print(f"\n全体 n={tot_n}  単勝blind ROI={tot_roi/tot_n:.3f}")
    print("読み: 正規implied（takeout除去）vs 実勝率の差が系統的（単調）なら人気-穴バイアス。"
          "単勝ROIは生オッズ（takeout込み）＝どの帯も<1なら市場は効率的で張る妙味なし。")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--tsv", required=True, help="純モデル(α=1.0) の --dump-features TSV（win_odds/着順列を使用）")
    run(ap.parse_args().tsv)


if __name__ == "__main__":
    main()
