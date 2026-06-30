#!/usr/bin/env python3
"""#272 改善① 素性重みスイープ: 純モデルの resolution を上げる重み構成を探す。

`feature_resolution_diag.py` の鏡映（raw_score→shrinkage→score_power→normalize→win_power・忠実性は
Rust `model_win` 列と一致, 旧 dump で 1.7e-16）を import し、`analyze backtest --dump-features` の TSV 上で
**重みだけ**を振って純モデルの resolution（AUC/top1）と calibration（Brier/LogLoss）を測る。Rust 変更・再 backtest 不要。
selection は screening 用途で、最終採用は実 backtest（pure resolution 改善＋blended 非回帰）で確定する
（pool AUC はレース間難易度差を混ぜるため、top1 も併記して判断する）。

診断（feature-resolution-diagnosis.md）の所見:
  - 最大重み 2.0 の course_gate が識別力ゼロ（レース内分散最小・複勝相関≒0）で希釈源。
  - 主シグナルは jockey_surface（ablation top1 −0.040）・trainer_surface。
本スクリプトでその所見を「重みを下げる/上げると resolution が上がるか」で定量化する。

within-race z-score（診断の指摘③）は **prototype 測定のみ**（Rust 実装は次ラウンド候補）。

使い方: python3 weight_sweep.py --tsv /tmp/pa/pure.tsv
"""

from __future__ import annotations

import argparse
import sys

import feature_resolution_diag as d


def gated_set(races):
    """(1) と同一母数: 勝馬記録あり かつ オッズあり のレース。"""
    g = set()
    for rid, rrows in races.items():
        yw = [1 if d._fp(r) == 1 else 0 for r in rrows]
        if any(yw) and d._implied(rrows)[1] > 0:
            g.add(rid)
    return g


def eval_weights(races, gated, weights=None):
    """重み構成での純モデル resolution/calibration を gated 母数で測る。"""
    aw, ay = [], []
    top1 = n = 0
    for rid, rrows in races.items():
        if rid not in gated:
            continue
        win_p, _, _ = d.race_probs(rrows, weights=weights)
        yw = [1 if d._fp(r) == 1 else 0 for r in rrows]
        n += 1
        if yw[d.argmax(win_p)] == 1:
            top1 += 1
        aw.extend(win_p)
        ay.extend(yw)
    return {"auc": d.auc(aw, ay), "top1": top1 / n if n else float("nan"),
            "brier": d.brier(aw, ay), "logloss": d.logloss(aw, ay), "n": n}


def zscore_win_scores(rrows, weights=None):
    """within-race z-score prototype: 各 factor の shrunk win レートをレース内で z 化し重み付き和。
    欠落 factor の馬はその項 0（レース内中立）。返すのは win スコア（順位用・確率でない）。"""
    n = len(rrows)
    score = [0.0] * n

    def accum(vals, ww):
        present = [v for v in vals if v is not None]
        if len(present) < 2:
            return
        m = sum(present) / len(present)
        sd = (sum((v - m) ** 2 for v in present) / len(present)) ** 0.5
        if sd == 0:
            return
        for i, v in enumerate(vals):
            if v is not None:
                score[i] += ww * (v - m) / sd

    for name, base, w in d.STAT_FACTORS:
        ww = w if weights is None else weights.get(name, w)
        if ww == 0.0:
            continue
        vals = []
        for r in rrows:
            fs = d._stat_cell(r, base)
            vals.append(None if fs is None else d.shrink_rate(fs[0], fs[3], d.PRIOR["win"]))
        accum(vals, ww)
    for name, idx, w in d.SCALAR_FACTORS:
        ww = w if weights is None else weights.get(name, w)
        if ww == 0.0:
            continue
        accum([d._scalar_cell(r, idx) for r in rrows], ww)
    return score


def eval_zscore(races, gated, weights=None):
    """z-score variant の resolution（AUC/top1 のみ・順位ベースで正規化非依存）。"""
    aw, ay = [], []
    top1 = n = 0
    for rid, rrows in races.items():
        if rid not in gated:
            continue
        sc = zscore_win_scores(rrows, weights)
        yw = [1 if d._fp(r) == 1 else 0 for r in rrows]
        n += 1
        if yw[d.argmax(sc)] == 1:
            top1 += 1
        aw.extend(sc)
        ay.extend(yw)
    return {"auc": d.auc(aw, ay), "top1": top1 / n if n else float("nan"), "n": n}


def fmt(label, m, base=None):
    s = f"  {label:32} AUC={m['auc']:.4f}  top1={m['top1']:.4f}  Brier={m.get('brier', float('nan')):.4f}  LogLoss={m.get('logloss', float('nan')):.4f}"
    if base is not None:
        s += f"   ΔAUC={m['auc']-base['auc']:+.4f}  Δtop1={m['top1']-base['top1']:+.4f}"
        if 'brier' in m and 'brier' in base:
            s += f"  ΔBrier={m['brier']-base['brier']:+.4f}"
    return s


def run(args):
    rows = d.load(args.tsv)
    races = d.group_by_race(rows)
    gated = gated_set(races)
    print(f"# {len(rows)} 行 / {len(races)} レース / gated {len(gated)} ({args.tsv})\n")

    if not gated:
        print("勝馬記録ありかつオッズありのレースが 0 件です。入力 TSV を確認してください。", file=sys.stderr)
        return
    # 注: baseline（weights=None）は現行ミラー重み＝マージ後は新重み（course_gate1.0/jockey2.0）を指す。
    # before/after の対比は (3) の candidates 行 "old (cg=2.0 jk=1.0)" / "cg=1.0 jk=2.0 (採用)" を使う。
    base = eval_weights(races, gated)  # 現行重み（weights=None）
    print("## baseline（現行重み）")
    print(fmt("baseline", base))
    print()

    # 1. course_gate 単独スイープ（診断の主目標）
    print("## (1) course_gate 重みスイープ（他は現行）")
    cg_results = {}
    for cg in [0.0, 0.5, 1.0, 1.5, 2.0]:
        m = eval_weights(races, gated, {"course_gate": cg})
        cg_results[cg] = m
        print(fmt(f"course_gate={cg}", m, base))
    best_cg = max(cg_results, key=lambda c: cg_results[c]["auc"])
    print(f"  -> AUC 最大は course_gate={best_cg}\n")

    # 2. best course_gate 固定で jockey/trainer を上げる
    print(f"## (2) course_gate={best_cg} 固定で jockey_surface / trainer_surface を up")
    for jw in [1.0, 1.5, 2.0]:
        for tw in [1.0, 1.5, 2.0]:
            w = {"course_gate": best_cg, "jockey_surface": jw, "trainer_surface": tw}
            m = eval_weights(races, gated, w)
            print(fmt(f"cg={best_cg} jk={jw} tr={tw}", m, base))
    print()

    # 3. 代表的な合成案
    print("## (3) 合成案")
    candidates = {
        "cg=1.0 jk=2.0 (採用/ADR0056)": {"course_gate": 1.0, "jockey_surface": 2.0},
        "cg=0 jk=2 tr=1.5": {"course_gate": 0.0, "jockey_surface": 2.0, "trainer_surface": 1.5},
        "cg=0.5 jk=1.5 tr=1.5": {"course_gate": 0.5, "jockey_surface": 1.5, "trainer_surface": 1.5},
        "cg=0 (drop) only": {"course_gate": 0.0},
        "stat-equal-1.0 (scalarは既定)": {n: 1.0 for n, *_ in d.STAT_FACTORS},
        # 旧重み（before baseline 再現用・ADR 0056 の 0.649/0.162 に対応）。
        "old (cg=2.0 jk=1.0)": {"course_gate": 2.0, "jockey_surface": 1.0},
    }
    best = (base, "baseline", None)
    for label, w in candidates.items():
        m = eval_weights(races, gated, w)
        print(fmt(label, m, base))
        if m["auc"] > best[0]["auc"]:
            best = (m, label, w)
    print(f"\n  -> AUC 最良: {best[1]}（{best[2]}）")

    # 4. 最良案の per-quarter 安定性
    if best[2] is not None:
        print(f"\n## (4) 最良案 [{best[1]}] の per-quarter 安定性（AUC/top1。base=現行ミラー重み）")
        print(f"  {'四半期':8} {'races':>6} {'AUC_base':>9} {'AUC_best':>9} {'top1_base':>10} {'top1_best':>10}")
        from collections import defaultdict
        q_races = defaultdict(dict)
        for rid, rrows in races.items():
            if rid not in gated:
                continue
            q_races[d.quarter(rrows[0][d.COL["date"]])][rid] = rrows
        for q in sorted(q_races):
            sub = q_races[q]
            sub_ids = set(sub)
            mb = eval_weights(sub, sub_ids)
            mx = eval_weights(sub, sub_ids, best[2])
            print(f"  {q:8} {len(sub):>6} {mb['auc']:>9.4f} {mx['auc']:>9.4f} {mb['top1']:>10.4f} {mx['top1']:>10.4f}")

    # 5. within-race z-score prototype（測るだけ・Rust 実装しない）
    print("\n## (5) within-race z-score prototype（resolution のみ・次ラウンド判断材料）")
    zb = eval_zscore(races, gated)
    print(f"  z-score(現行重み)        AUC={zb['auc']:.4f}  top1={zb['top1']:.4f}  (baseline AUC={base['auc']:.4f} top1={base['top1']:.4f})")
    if best[2] is not None:
        zx = eval_zscore(races, gated, best[2])
        print(f"  z-score(最良重み {best[1]})  AUC={zx['auc']:.4f}  top1={zx['top1']:.4f}")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--tsv", required=True, help="純モデル(α=1.0) の --dump-features TSV")
    run(ap.parse_args())


if __name__ == "__main__":
    main()
