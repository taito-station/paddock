#!/usr/bin/env python3
"""#272 改善② 欠落補完プロトタイプ: 高欠落 stat factor の欠落処理を変えて純 resolution を測る。

診断（feature_resolution_diag.py (2)(3)）の所見:
  - horse_surface(欠落0.285)/horse_distance(0.351)/horse_track_condition(0.393) は corr≈0.24-0.26 と
    識別力があるのに ablation で外しても top1 が変わらない〜僅かに改善。
  - 原因は「欠落 factor を weight から落とす」現状処理。持つ馬だけがシグナルを得て、欠落馬との
    レース内相対比較が失われる（識別力が希釈される）。

本スクリプトは `feature_resolution_diag.py` の鏡映パイプラインを再利用し、raw_score の欠落処理だけを
差し替えて resolution（AUC/top1/Brier/LogLoss）を gated 母数＋四半期別で測る。Rust 変更・再 backtest 不要。
selection は screening 用途で、最終採用は実 backtest（pure resolution 改善＋blended 非回帰）で確定する。

補完戦略:
  drop           : 欠落 factor を落とす（現状 = baseline）
  prior          : 欠落を pure prior（starts=0 相当）で埋め、weight を数える
  race_mean      : 欠落を同レース内 present 馬の shrunk レート平均で埋める（present<2 は prior）

対象 factor 集合を --targets で選ぶ（既定 = 高欠落3factor）。全 stat は "all"。

使い方: python3 impute_prototype.py --tsv /tmp/pa/pure.tsv
"""

from __future__ import annotations

import argparse
from collections import defaultdict

import feature_resolution_diag as d

HIGH_MISS = ["horse_surface", "horse_distance", "horse_track_condition"]
ALL_STAT = [n for n, *_ in d.STAT_FACTORS]


def race_raw_scores(rrows, sel, impute="drop", targets=frozenset(), weights=None):
    """1 レース分の raw_score(sel) を欠落補完付きで返す。

    impute=drop は現行 raw_score と厳密一致（targets 無視）。prior/race_mean は
    targets の stat factor のみ補完（他 factor・scalar は従来通り落とす）。
    """
    sel_i = {"win": 0, "place": 1, "show": 2}[sel]
    n = len(rrows)

    # 各 target factor のレース内 present 平均（race_mean 用）を先に計算。
    race_mean = {}
    if impute == "race_mean":
        for name, base, _ in d.STAT_FACTORS:
            if name not in targets:
                continue
            present = []
            for r in rrows:
                fs = d._stat_cell(r, base)
                if fs is not None:
                    present.append(d.shrink_rate(fs[sel_i], fs[3], d.PRIOR[sel]))
            race_mean[name] = (sum(present) / len(present)) if len(present) >= 2 else d.PRIOR[sel]

    out = [0.0] * n
    for i, r in enumerate(rrows):
        weighted = weight = 0.0
        for name, base, w in d.STAT_FACTORS:
            ww = w if weights is None else weights.get(name, w)
            if ww == 0.0:
                continue
            fs = d._stat_cell(r, base)
            if fs is None:
                if impute == "drop" or name not in targets:
                    continue
                val = d.PRIOR[sel] if impute == "prior" else race_mean[name]
            else:
                val = d.shrink_rate(fs[sel_i], fs[3], d.PRIOR[sel])
            weighted += ww * val
            weight += ww
        for name, idx, w in d.SCALAR_FACTORS:
            ww = w if weights is None else weights.get(name, w)
            if ww == 0.0:
                continue
            v = d._scalar_cell(r, idx)
            if v is None:
                continue
            weighted += ww * v
            weight += ww
        out[i] = 0.0 if weight == 0.0 else weighted / weight
    return out


def race_probs_impute(rrows, impute="drop", targets=frozenset(), weights=None):
    """race_probs の欠落補完版。drop+空targets は現行 race_probs と一致（忠実性で担保）。"""
    win_s = race_raw_scores(rrows, "win", impute, targets, weights)
    place_s = d.score_power(race_raw_scores(rrows, "place", impute, targets, weights), d.PLACE_SHOW_POWER)
    show_s = d.score_power(race_raw_scores(rrows, "show", impute, targets, weights), d.PLACE_SHOW_POWER)
    win_p = d.normalize_to_sum(win_s, 1.0)
    place_p = d.normalize_to_sum(place_s, 2.0)
    show_p = d.normalize_to_sum(show_s, 3.0)
    for i in range(len(rrows)):
        place_p[i] = min(max(place_p[i], win_p[i]), 1.0)
        show_p[i] = min(max(show_p[i], place_p[i]), 1.0)
    win_p = d.win_power(win_p, d.WIN_POWER)
    for i in range(len(rrows)):
        place_p[i] = min(max(place_p[i], win_p[i]), 1.0)
        show_p[i] = min(max(show_p[i], place_p[i]), 1.0)
    return win_p, place_p, show_p


def gated_set(races):
    g = set()
    for rid, rrows in races.items():
        yw = [1 if d._fp(r) == 1 else 0 for r in rrows]
        if any(yw) and d._implied(rrows)[1] > 0:
            g.add(rid)
    return g


def evaluate(races, gated, impute="drop", targets=frozenset(), weights=None):
    aw, ay = [], []
    top1 = n = 0
    for rid, rrows in races.items():
        if rid not in gated:
            continue
        win_p, _, _ = race_probs_impute(rrows, impute, targets, weights)
        yw = [1 if d._fp(r) == 1 else 0 for r in rrows]
        n += 1
        if yw[d.argmax(win_p)] == 1:
            top1 += 1
        aw.extend(win_p)
        ay.extend(yw)
    return {"auc": d.auc(aw, ay), "top1": top1 / n if n else float("nan"),
            "brier": d.brier(aw, ay), "logloss": d.logloss(aw, ay), "n": n}


def per_quarter(races, gated, impute, targets, weights=None):
    q_races = defaultdict(dict)
    for rid, rrows in races.items():
        if rid not in gated:
            continue
        q_races[d.quarter(rrows[0][d.COL["date"]])][rid] = rrows
    res = {}
    for q in sorted(q_races):
        sub = q_races[q]
        res[q] = evaluate(sub, set(sub), impute, targets, weights)
    return res


def fmt(label, m, base):
    s = f"  {label:34} AUC={m['auc']:.4f}  top1={m['top1']:.4f}  Brier={m['brier']:.4f}  LogLoss={m['logloss']:.4f}"
    s += f"   ΔAUC={m['auc']-base['auc']:+.4f}  Δtop1={m['top1']-base['top1']:+.4f}  ΔBrier={m['brier']-base['brier']:+.4f}"
    return s


def verify_dump(args):
    """production 実装（--impute-missing-factors）で書き出した dump の model_win 列が、本プロトタイプの
    race_mean[all stat] 補完と一致するか（忠実性アンカー）を確認する。ADR 0057 の Rust 実装が
    screening で測った改善を忠実に実現していることの担保。"""
    rows = d.load(args.tsv)
    races = d.group_by_race(rows)
    fid = 0.0
    for rid, rrows in races.items():
        win_p, _, _ = race_probs_impute(rrows, "race_mean", frozenset(ALL_STAT))
        for i, r in enumerate(rrows):
            fid = max(fid, abs(win_p[i] - float(r[d.COL["model_win"]])))
    ok = fid < 1e-6
    print(f"# 忠実性アンカー（race_mean[all stat] ≡ dump model_win）: max|Δ| = {fid:.2e}  -> "
          f"{'OK' if ok else 'FAIL（Rust 実装とプロトタイプが不一致）'}")


def run(args):
    if args.verify_dump:
        verify_dump(args)
        return
    rows = d.load(args.tsv)
    races = d.group_by_race(rows)
    gated = gated_set(races)
    print(f"# {len(rows)} 行 / {len(races)} レース / gated {len(gated)} ({args.tsv})\n")

    # 忠実性: drop+空targets が現行 race_probs と一致することを確認（プロトタイプの前提）。
    fid = 0.0
    for rid, rrows in races.items():
        a, _, _ = race_probs_impute(rrows, "drop")
        b, _, _ = d.race_probs(rrows)
        fid = max(fid, max(abs(x - y) for x, y in zip(a, b)))
    print(f"## 忠実性（drop ≡ 現行 race_probs）: max|Δ| = {fid:.2e}  -> {'OK' if fid < 1e-12 else 'FAIL'}\n")

    base = evaluate(races, gated, "drop")
    print("## baseline（drop = 現状）")
    print(f"  baseline                           AUC={base['auc']:.4f}  top1={base['top1']:.4f}  "
          f"Brier={base['brier']:.4f}  LogLoss={base['logloss']:.4f}\n")

    combos = [
        ("prior  [horse_surface]", "prior", ["horse_surface"]),
        ("prior  [horse_distance]", "prior", ["horse_distance"]),
        ("prior  [horse_track_condition]", "prior", ["horse_track_condition"]),
        ("prior  [high-miss 3]", "prior", HIGH_MISS),
        ("prior  [all stat]", "prior", ALL_STAT),
        ("race_mean [horse_surface]", "race_mean", ["horse_surface"]),
        ("race_mean [horse_distance]", "race_mean", ["horse_distance"]),
        ("race_mean [horse_track_condition]", "race_mean", ["horse_track_condition"]),
        ("race_mean [high-miss 3]", "race_mean", HIGH_MISS),
        ("race_mean [all stat]", "race_mean", ALL_STAT),
    ]
    print("## 補完戦略 × 対象 factor（gated 母数）")
    results = {}
    for label, imp, tg in combos:
        m = evaluate(races, gated, imp, frozenset(tg))
        results[label] = (m, imp, tg)
        print(fmt(label, m, base))
    print()

    # top1 で最良を選ぶ（resolution の主指標）。AUC タイブレーク。
    best_label = max(results, key=lambda k: (round(results[k][0]["top1"], 4), results[k][0]["auc"]))
    bm, bimp, btg = results[best_label]
    print(f"  -> top1 最良: {best_label}  (top1={bm['top1']:.4f} vs base {base['top1']:.4f})\n")

    if bm["top1"] > base["top1"] or bm["auc"] > base["auc"]:
        print(f"## 最良案 [{best_label}] の四半期安定性")
        print(f"  {'四半期':8} {'races':>6} {'top1_base':>10} {'top1_best':>10} {'AUC_base':>9} {'AUC_best':>9}")
        pb = per_quarter(races, gated, "drop", frozenset())
        px = per_quarter(races, gated, bimp, frozenset(btg))
        for q in sorted(pb):
            print(f"  {q:8} {pb[q]['n']:>6} {pb[q]['top1']:>10.4f} {px[q]['top1']:>10.4f} "
                  f"{pb[q]['auc']:>9.4f} {px[q]['auc']:>9.4f}")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--tsv", required=True, help="純モデル(α=1.0) の --dump-features TSV")
    ap.add_argument("--verify-dump", action="store_true",
                    help="dump の model_win が race_mean[all stat] 補完と一致するか（忠実性）を確認する")
    run(ap.parse_args())


if __name__ == "__main__":
    main()
