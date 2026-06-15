#!/usr/bin/env python3
"""買い方（馬券構成）別の回収率を確定配当で評価する戦略ハーネス（#122）.

predict 本命を軸にした「単勝のみ / 馬連＋ワイド流し / ＋三連複流し」を予算内・100 円単位で
組み立て、netkeiba 確定配当（fetch_payouts.py の payouts.json）で精算し、戦略別の
回収率・収支を比較する。買い方で回収率が大きく変わる（issue #122）ことを再現・定量化する。

精算は payouts.json（確定した的中組のみが入る）への組番一致で判定する。確定配当そのものが
「どの組が当たったか」を持つため、着順から的中を再導出する必要はない（同着・複数本ワイドも自然対応）。

使い方:
    python3 strategy_eval.py preds.json payouts.json [options]

options:
    --budget N       1 レースの予算（円, 既定 5000）
    --partners K     相手頭数。カンマ区切りで複数指定すると感度テーブルを出す（既定 5）
    --alloc q,w,t    馬連:ワイド:三連複 の予算配分の相対重み（既定 1,1,1）
    --axis {model,market}  軸の選び方（既定 model=予想本命 / market=1番人気）
    --win-odds FILE  --axis market 用の win_odds.csv（answer_check.py と同形式）

例:
    python3 strategy_eval.py preds.json payouts.json --budget 5000 --partners 3,5,7
"""
import sys
import json
import csv
import argparse
from collections import defaultdict
from itertools import combinations

SLUG2JP = {"tokyo": "東京", "hanshin": "阪神", "kyoto": "京都", "nakayama": "中山",
           "chukyo": "中京", "sapporo": "札幌", "hakodate": "函館", "fukushima": "福島",
           "niigata": "新潟", "kokura": "小倉"}


def parse_args(argv):
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("preds")
    ap.add_argument("payouts")
    ap.add_argument("--budget", type=int, default=5000)
    ap.add_argument("--partners", default="5")
    ap.add_argument("--alloc", default="1,1,1")
    ap.add_argument("--axis", choices=["model", "market"], default="model")
    ap.add_argument("--win-odds", dest="win_odds")
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__, file=sys.stderr)
        sys.exit(0 if argv else 1)
    return ap.parse_args(argv)


def load_market_fav(path):
    """win_odds.csv（race_id, combination_key, popularity, odds）→ {(venue_jp, race_num): 軸馬番}.

    ORDER BY popularity 前提で 1 番人気のみ採用（後勝ち上書きを避ける）。answer_check.py と同方針。
    """
    fav = {}
    with open(path, encoding="utf-8") as f:
        for row in csv.reader(f):
            if len(row) < 4:
                continue
            try:
                rid, num, pop = row[0], int(row[1]), row[2].strip()
                parts = rid.split("-")  # YYYY-round-slug-day-RRR
                key = (SLUG2JP.get(parts[2], parts[2]), int(parts[4].replace("R", "")))
                popi = int(float(pop))
            except (ValueError, IndexError):
                continue
            if popi == 1 and key not in fav:
                fav[key] = num
    return fav


def code_unordered(nums):
    """無順の組番コード（昇順 `-` 連結）。fetch_payouts の正規化と一致させる。"""
    return "-".join(str(x) for x in sorted(nums))


def distribute(type_budget, n_combos):
    """type_budget を n_combos 点に 100 円単位で均等配分する。

    全点 100 円すら賄えない予算なら、賄える点数ぶんだけ 100 円ずつ張る（残りは 0＝買わない）。
    返り値: 各点の賭け金リスト（合計は type_budget 以下）。
    """
    if n_combos <= 0 or type_budget < 100:
        return [0] * max(n_combos, 0)
    per = (type_budget // n_combos // 100) * 100
    if per >= 100:
        return [per] * n_combos
    k = type_budget // 100  # 100 円で買える点数
    return [100] * k + [0] * (n_combos - k)


def build_bets(axis, partners, budget, alloc, with_trio):
    """軸流しの買い目（(type_label, combination_code, stake) のリスト）を組み立てる。

    with_trio=False: 馬連流し＋ワイド流し。True: さらに三連複（軸 1 頭流し formation）。
    予算は alloc の重みで券種に配分し、券種内は distribute で 100 円単位均等配分する。
    """
    qn = [code_unordered([axis, p]) for p in partners]                 # 馬連: 軸-相手 K 点
    wd = list(qn)                                                       # ワイド: 同じ組（K 点）
    tr = [code_unordered([axis, a, b]) for a, b in combinations(partners, 2)]  # 三連複 C(K,2)

    wq, ww, wt = alloc
    if not with_trio:
        wt = 0
    total_w = wq + ww + wt
    if total_w == 0:
        return []

    def type_budget(w):
        return (budget * w // total_w // 100) * 100

    bets = []
    for code, st in zip(qn, distribute(type_budget(wq), len(qn))):
        if st:
            bets.append(("quinella", code, st))
    for code, st in zip(wd, distribute(type_budget(ww), len(wd))):
        if st:
            bets.append(("wide", code, st))
    if with_trio:
        for code, st in zip(tr, distribute(type_budget(wt), len(tr))):
            if st:
                bets.append(("trio", code, st))
    return bets


def settle(bets, payouts):
    """買い目を確定配当で精算し (賭け金合計, 払戻合計) を返す。"""
    stake = sum(st for _, _, st in bets)
    payout = 0
    for label, code, st in bets:
        won = payouts.get(label, {}).get(code)
        if won:
            payout += st // 100 * won
    return stake, payout


# 評価する戦略。axis/partners/budget/alloc を受けて買い目リストを返す。
STRATEGIES = [
    ("本命単勝のみ",
     lambda axis, partners, b, alloc: [("win", str(axis), (b // 100) * 100)]),
    ("本命軸 馬連+ワイド流し",
     lambda axis, partners, b, alloc: build_bets(axis, partners, b, alloc, with_trio=False)),
    ("本命軸 馬連+ワイド+三連複流し",
     lambda axis, partners, b, alloc: build_bets(axis, partners, b, alloc, with_trio=True)),
]


def main(argv):
    args = parse_args(argv)
    with open(args.preds, encoding="utf-8") as f:
        preds = json.load(f)
    with open(args.payouts, encoding="utf-8") as f:
        payout_races = json.load(f)
    alloc = tuple(int(x) for x in args.alloc.split(","))
    if len(alloc) != 3:
        print("--alloc は 3 値（馬連,ワイド,三連複）", file=sys.stderr)
        sys.exit(1)
    ks = [int(x) for x in args.partners.split(",")]
    fav = {}
    if args.axis == "market":
        if not args.win_odds:
            print("--axis market には --win-odds が必要", file=sys.stderr)
            sys.exit(1)
        fav = load_market_fav(args.win_odds)

    # payouts を (venue_jp, race_num) で索く。空（中止/全馬取消）レースは除外。
    pay_idx = {(p["venue_jp"], p["race_num"]): p["payouts"]
               for p in payout_races if p.get("payouts")}

    # 予想本命（モデル）・相手ランキングを (venue, race_num) で用意。
    ranked = {}
    for r in preds:
        horses = sorted(r["horses"], key=lambda h: h["win"], reverse=True)
        if horses:
            ranked[(r["venue"], r["race_num"])] = [h["num"] for h in horses]

    # 各 (budget, K, strategy) で全レース合算の stake/payout を集計。
    # agg[K][strategy_name] -> [total_stake, total_payout, n_races]
    agg = {k: {name: [0, 0, 0] for name, _ in STRATEGIES} for k in ks}
    used_races = set()
    for key, order in ranked.items():
        payouts = pay_idx.get(key)
        if not payouts:
            continue  # 確定配当が無い（未取得/中止）レースは母数から落とす
        if args.axis == "market":
            axis = fav.get(key)
            if axis is None:
                continue
        else:
            axis = order[0]
        used_races.add(key)
        for k in ks:
            partners = [n for n in order if n != axis][:k]
            for name, builder in STRATEGIES:
                bets = builder(axis, partners, args.budget, alloc)
                stake, payout = settle(bets, payouts)
                cell = agg[k][name]
                cell[0] += stake
                cell[1] += payout
                cell[2] += 1

    # 出力。
    print(f"予算: ¥{args.budget}/R  軸: {args.axis}  配分(馬連:ワイド:三連複)={':'.join(map(str, alloc))}")
    print(f"評価レース数: {len(used_races)}")
    print()
    if len(ks) == 1:
        k = ks[0]
        print(f"相手頭数: {k}")
        print(f"{'戦略':<28}{'回収率':>8}{'収支':>12}{'賭け計':>12}{'払戻計':>12}")
        for name, _ in STRATEGIES:
            st, pay, _n = agg[k][name]
            roi = pay / st * 100 if st else 0.0
            print(f"{name:<28}{roi:>7.1f}%{pay - st:>+12,}{st:>12,}{pay:>12,}")
    else:
        # 感度テーブル: 行=戦略, 列=相手頭数 K の回収率。
        header = "".join(f"K={k:<7}" for k in ks)
        print(f"相手頭数感度（回収率）\n{'戦略':<28}{header}")
        for name, _ in STRATEGIES:
            cells = ""
            for k in ks:
                st, pay, _n = agg[k][name]
                roi = pay / st * 100 if st else 0.0
                cells += f"{roi:>6.1f}% "
            print(f"{name:<28}{cells}")


if __name__ == "__main__":
    main(sys.argv[1:])
