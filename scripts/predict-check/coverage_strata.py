#!/usr/bin/env python3
"""#272 追測: coverage を上げれば純モデルの resolution は改善するか（本番非変更）。

「馬履歴 factor（horse_surface/horse_distance/horse_track_condition）が現行 ~60-71% しか
乗っていないので、全 runner の履歴を大量 fetch して coverage を上げれば resolution が伸びるのでは」
という仮説を、既存 dump だけで測って否定するツール。ADR 0058 の「coverage cap」誤診の訂正根拠。

gated レース（勝馬記録あり かつ オッズあり・feature_resolution_diag と同一母数）を
「1 レース内の馬履歴 factor カバー率」で層別し、各層でモデル(純 α=1.0) vs 市場の AUC/top1 を出す。
高カバー層でモデルが市場に近づけば coverage が lever。頭打ちなら factor 自体が天井（＝冗長）。

確率推定は feature_resolution_diag.py の Rust 鏡映を import して忠実性を担保する（標準ライブラリのみ）。
course_gate(汎用コース×枠バイアス・馬履歴不要)・jockey/trainer(騎手・厩舎キー) は coverage の分子から除外。

使い方:
  python3 coverage_strata.py --tsv /tmp/pa/pure.tsv
"""
from __future__ import annotations
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import feature_resolution_diag as D  # noqa: E402

# 真に馬履歴依存な factor＝ horse_surface(7)/horse_distance(11)/horse_track_condition(23)。
# course_gate(3) は course×gate の汎用バイアスで馬履歴不要（present ~96%）なので分子から除外。
HORSE_BASES = [7, 11, 23]


def has_horse_history(row):
    return any(D._stat_cell(row, base) is not None for base in HORSE_BASES)


def stratum(cov):
    if cov == 0:
        return "0%"
    if cov < 0.25:
        return "0-25%"
    if cov < 0.50:
        return "25-50%"
    if cov < 0.75:
        return "50-75%"
    if cov < 1.0:
        return "75-99%"
    return "100%"


ORDER = ["0%", "0-25%", "25-50%", "50-75%", "75-99%", "100%"]


def run(tsv):
    rows = D.load(tsv)
    races = D.group_by_race(rows)
    S = {k: {"n": 0, "t1m": 0, "t1k": 0, "mw": [], "iw": [], "y": [], "field": []} for k in ORDER}

    fid_max = 0.0  # 忠実性アンカー: mirror 再計算 win が dump の model_win 列と一致するか（sibling 準拠）
    for _rid, rr in races.items():
        win_p, _, _ = D.race_probs(rr)
        for i, r in enumerate(rr):  # gate 前・全行で測る（鏡映の正しさは outcome 非依存）
            fid_max = max(fid_max, abs(win_p[i] - float(r[D.COL["model_win"]])))
        fp = [D._fp(r) for r in rr]
        y = [1 if p == 1 else 0 for p in fp]
        implied, s = D._implied(rr)
        if not (any(y) and s > 0):  # feature_resolution_diag と同一 gated 母数
            continue
        cov = sum(1 for r in rr if has_horse_history(r)) / len(rr)
        b = S[stratum(cov)]
        b["n"] += 1
        b["field"].append(len(rr))
        if y[D.argmax(win_p)] == 1:
            b["t1m"] += 1
        if y[D.argmax(implied)] == 1:
            b["t1k"] += 1
        b["mw"].extend(win_p); b["iw"].extend(implied); b["y"].extend(y)

    if fid_max >= 1e-6:
        # 鏡映が dump と一致しない＝非 production flags（m≠10 等）で生成された dump の疑い。
        # 黙って誤った層別を出さず打ち切る（production 相当: --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0）。
        print(f"忠実性 FAIL: max|python_win - dump model_win| = {fid_max:.2e} >= 1e-6。"
              f"production 相当 flags の純 dump か確認（中断）。", file=sys.stderr)
        sys.exit(1)
    print(f"# 忠実性アンカー max|python_win - dump model_win| = {fid_max:.2e} (OK)\n")

    print(f"{'馬履歴cov層':10} {'races':>6} {'頭数':>5} {'top1_model':>11} {'top1_market':>12} "
          f"{'AUC_model':>10} {'AUC_market':>11} {'gap':>7}")
    tot = 0
    for k in ORDER:
        b = S[k]
        if b["n"] == 0:
            continue
        tot += b["n"]
        am, ak = D.auc(b["mw"], b["y"]), D.auc(b["iw"], b["y"])
        fld = sum(b["field"]) / len(b["field"])
        print(f"{k:10} {b['n']:>6} {fld:>5.1f} {b['t1m']/b['n']:>11.3f} {b['t1k']/b['n']:>12.3f} "
              f"{am:>10.3f} {ak:>11.3f} {ak-am:>7.3f}")
    print(f"\n全 gated races={tot}")
    print("読み: model AUC が層でフラット＝馬履歴 factor は常在信号(course_gate/jockey/trainer)に冗長。"
          "coverage を上げても resolution は伸びない（天井は coverage でなく factor 冗長性）。"
          "※層はレース母集団が非同質（0% 層は新馬等に偏りうる）＝端点比較は交絡込みの directional read。"
          "交絡なしの根拠は ADR 0057 の ablation（馬 factor 除去でも top1 ほぼ不変）。")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--tsv", required=True, help="純モデル(α=1.0) の --dump-features TSV")
    run(ap.parse_args().tsv)


if __name__ == "__main__":
    main()
