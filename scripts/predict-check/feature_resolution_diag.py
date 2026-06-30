#!/usr/bin/env python3
"""#272 Phase A 診断: 純モデル確率の素性分解（resolution か calibration かを確定）。

`paddock-analyze backtest --blend-alpha 1.0 ... --dump-features <tsv>` の出力（as-of・リーク無し）
を入力に、純モデルの確率がフラットな原因を素性レベルで診断する。production コードは触らず、
Rust の確率推定パイプラインを Python で忠実に鏡映（fidelity check で担保）して解析する。
依存は標準ライブラリのみ（既存 scripts/predict-check の流儀・in-house 方針）。

出す診断:
  (0) 忠実性アンカー: Python 再構成の model_win が dump の model_win 列と一致するか（全数値の前提）。
  (1) resolution: レース内 argmax 的中・Spearman 順位相関・AUC を純モデルと市場で並置。
  (2) 素性別識別力: 各 factor のレース内分散と outcome 相関、欠落率。
  (3) leave-one-out ablation: factor を 1 つ外したときの Brier/LogLoss/top1 の Δ。
  (4) 正規化の圧縮度: var(raw_score) vs var(model_win)。
  (5) isotonic 上限: walk-forward（前窓 fit→後窓適用）で isotonic を当て、市場との gap をどこまで詰めるか。

判定: isotonic で Brier が市場≈まで縮む→calibration 限定（isotonic 実装）。残る→resolution 限定（素性改善）。

使い方:
  python3 feature_resolution_diag.py --tsv /tmp/pa/pure.tsv
"""

from __future__ import annotations

import argparse
import bisect
import math
import statistics
import sys
from collections import defaultdict

# --- Rust 定数の鏡映（src/domain/src/prediction/weights.rs, config.rs） -------------
# FactorStat 系（shrinkage 対象）。名前→(列の起点 index, 重み)。各 factor は win/place/show/starts の 4 列。
STAT_FACTORS = [
    ("course_gate", 3, 2.0),
    ("horse_surface", 7, 1.0),
    ("horse_distance", 11, 1.0),
    ("jockey_surface", 15, 1.0),
    ("trainer_surface", 19, 1.0),
    ("horse_track_condition", 23, 1.0),
]
# スカラー系（shrinkage なし・win/place/show 同値）。名前→(列 index, 重み)。
SCALAR_FACTORS = [
    ("recent_form", 27, 0.25),
    ("weight_carried", 28, 0.25),
    ("jockey_recent_form", 29, 0.0),  # 無効（ADR 0038）。weight 0 で母数に入らない。
]
PRIOR = {"win": 1.0 / 14.0, "place": 2.0 / 14.0, "show": 3.0 / 14.0}
SHRINKAGE_M = 10.0
WIN_POWER = 1.25
PLACE_SHOW_POWER = 2.0
# γ=1.0 を厳密 no-op 扱いにする machine-eps（Rust estimate.rs の f64::EPSILON 判定の鏡映）。
GAMMA_EPS = 2.2e-16

COL = {
    "race_id": 0, "date": 1, "horse_num": 2,
    "model_win": 30, "model_place": 31, "model_show": 32,
    "finishing_position": 33, "win_odds": 34, "popularity": 35,
}
N_COLS = 36


# --- パイプライン鏡映 -------------------------------------------------------------
def shrink_rate(rate, starts, prior, m=SHRINKAGE_M):
    return (starts * rate + m * prior) / (starts + m)


def _stat_cell(row, base):
    """FactorStat 1 つを (win, place, show, starts) で返す。欠落（空セル）は None。
    Rust の dump は factor 単位で 4 セル all-or-nothing だが、一部だけ空の壊れた行でも
    float("") の ValueError を出さず欠落扱いにする（防御的・4 セルいずれか空なら None）。"""
    if any(row[base + k] == "" for k in range(4)):
        return None
    return (float(row[base]), float(row[base + 1]), float(row[base + 2]), int(row[base + 3]))


def _scalar_cell(row, idx):
    return None if row[idx] == "" else float(row[idx])


def raw_score(row, sel, drop=None):
    """Rust raw_score の鏡映。sel ∈ {win,place,show}。drop=factor 名で leave-one-out。"""
    sel_i = {"win": 0, "place": 1, "show": 2}[sel]
    weighted = 0.0
    weight = 0.0
    for name, base, w in STAT_FACTORS:
        if name == drop:
            continue
        fs = _stat_cell(row, base)
        if fs is None:
            continue
        val = shrink_rate(fs[sel_i], fs[3], PRIOR[sel])
        weighted += w * val
        weight += w
    for name, idx, w in SCALAR_FACTORS:
        if name == drop or w == 0.0:
            continue
        v = _scalar_cell(row, idx)
        if v is None:
            continue
        weighted += w * v
        weight += w
    return 0.0 if weight == 0.0 else weighted / weight


def score_power(scores, gamma):
    if gamma is None or not math.isfinite(gamma) or gamma <= 0.0 or abs(gamma - 1.0) < GAMMA_EPS:
        return scores
    return [s ** gamma for s in scores]


def normalize_to_sum(scores, target):
    n = len(scores)
    total = sum(scores)
    if total <= 0.0:
        return [min(target / n, 1.0)] * n
    return [min(s / total * target, 1.0) for s in scores]


def win_power(win_probs, gamma):
    if gamma is None or not math.isfinite(gamma) or gamma <= 0.0 or abs(gamma - 1.0) < GAMMA_EPS:
        return list(win_probs)
    powered = [p ** gamma for p in win_probs]
    total = sum(powered)
    if total <= 0.0 or not math.isfinite(total):
        return list(win_probs)
    return [min(p / total, 1.0) for p in powered]


def race_probs(rows, drop=None):
    """1 レース分の最終 win/place/show（純モデル α=1.0）を Rust 同手順で出す。"""
    win_s = [raw_score(r, "win", drop) for r in rows]
    place_s = score_power([raw_score(r, "place", drop) for r in rows], PLACE_SHOW_POWER)
    show_s = score_power([raw_score(r, "show", drop) for r in rows], PLACE_SHOW_POWER)
    win_p = normalize_to_sum(win_s, 1.0)
    place_p = normalize_to_sum(place_s, 2.0)
    show_p = normalize_to_sum(show_s, 3.0)
    for i in range(len(rows)):
        place_p[i] = min(max(place_p[i], win_p[i]), 1.0)
        show_p[i] = min(max(show_p[i], place_p[i]), 1.0)
    # blend は α=1.0 で no-op。win_power をブレンド後 win に適用し再正規化＋単調再是正。
    win_p = win_power(win_p, WIN_POWER)
    for i in range(len(rows)):
        place_p[i] = min(max(place_p[i], win_p[i]), 1.0)
        show_p[i] = min(max(show_p[i], place_p[i]), 1.0)
    return win_p, place_p, show_p


# --- メトリクス（stdlib のみ） ----------------------------------------------------
def mean(xs):
    xs = list(xs)
    return sum(xs) / len(xs) if xs else float("nan")


def nanmean(xs):
    xs = [x for x in xs if x == x]  # NaN 除外
    return mean(xs) if xs else float("nan")


def pvar(xs):
    xs = list(xs)
    return statistics.pvariance(xs) if len(xs) >= 1 else 0.0


def brier(p, y):
    return mean((pi - yi) ** 2 for pi, yi in zip(p, y))


def logloss(p, y, eps=1e-15):
    return -mean(
        yi * math.log(min(max(pi, eps), 1 - eps)) + (1 - yi) * math.log(1 - min(max(pi, eps), 1 - eps))
        for pi, yi in zip(p, y)
    )


def _rankdata(a):
    """平均ランク（同値補正）。"""
    idx = sorted(range(len(a)), key=lambda i: a[i])
    ranks = [0.0] * len(a)
    i = 0
    while i < len(a):
        j = i
        while j + 1 < len(a) and a[idx[j + 1]] == a[idx[i]]:
            j += 1
        avg = (i + j) / 2.0 + 1.0  # 1-based 平均ランク
        for k in range(i, j + 1):
            ranks[idx[k]] = avg
        i = j + 1
    return ranks


def auc(scores, y):
    """二値 y に対する AUC（Mann-Whitney, 同値 0.5）。"""
    npos = sum(1 for v in y if v == 1)
    nneg = len(y) - npos
    if npos == 0 or nneg == 0:
        return float("nan")
    ranks = _rankdata(scores)
    r_pos = sum(r for r, yi in zip(ranks, y) if yi == 1)
    return (r_pos - npos * (npos + 1) / 2.0) / (npos * nneg)


def corr(a, b):
    if len(a) < 2:
        return float("nan")
    sa, sb = pvar(a) ** 0.5, pvar(b) ** 0.5
    if sa == 0 or sb == 0:
        return float("nan")
    ma, mb = mean(a), mean(b)
    cov = mean((ai - ma) * (bi - mb) for ai, bi in zip(a, b))
    return cov / (sa * sb)


def spearman(a, b):
    if len(a) < 2:
        return float("nan")
    return corr(_rankdata(a), _rankdata(b))


def argmax(xs):
    return max(range(len(xs)), key=lambda i: xs[i])


def pava_fit(x, y):
    """Pool-Adjacent-Violators で単調増加 isotonic を fit。(thresholds_x, fitted_y) を返す。"""
    order = sorted(range(len(x)), key=lambda i: x[i])
    merged = []  # [value, weight, x_right]
    for i in order:
        merged.append([float(y[i]), 1.0, x[i]])
        while len(merged) > 1 and merged[-2][0] > merged[-1][0]:
            v2, w2, x2 = merged.pop()
            v1, w1, _ = merged.pop()
            nw = w1 + w2
            merged.append([(v1 * w1 + v2 * w2) / nw, nw, x2])
    thr_x = [b[2] for b in merged]
    fit_y = [b[0] for b in merged]
    return thr_x, fit_y


def pava_apply(thr_x, fit_y, x):
    i = bisect.bisect_left(thr_x, x)
    if i >= len(fit_y):
        i = len(fit_y) - 1
    return fit_y[i]


# --- データ読み込み --------------------------------------------------------------
def load(tsv):
    rows, skipped = [], 0
    with open(tsv, encoding="utf-8") as f:
        header = f.readline().rstrip("\n").split("\t")
        assert len(header) == N_COLS, f"列数 {len(header)} != {N_COLS}"
        for line in f:
            cells = line.rstrip("\n").split("\t")
            if len(cells) == N_COLS:
                rows.append(cells)
            else:
                skipped += 1
    if skipped:
        # 列数不一致行を黙って捨てると schema 変更・破損に気づけない。件数を必ず可視化する。
        print(f"警告: 列数 != {N_COLS} の行を {skipped} 件スキップしました（dump の整合性を確認）。",
              file=sys.stderr)
    return rows


def group_by_race(rows):
    races = defaultdict(list)
    for r in rows:
        races[r[COL["race_id"]]].append(r)
    return races


def quarter(date_str):
    y, m, _ = date_str.split("-")
    return f"{y}Q{(int(m) - 1) // 3 + 1}"


def _fp(r):
    v = r[COL["finishing_position"]]
    return int(v) if v else 99


def _implied(rows):
    odds = [float(r[COL["win_odds"]]) if r[COL["win_odds"]] else None for r in rows]
    raw = [1.0 / o if o and o >= 1.0 else 0.0 for o in odds]
    s = sum(raw)
    return [x / s if s > 0 else 0.0 for x in raw], s


# --- 診断本体 --------------------------------------------------------------------
def run(args):
    rows = load(args.tsv)
    races = group_by_race(rows)
    print(f"# 読み込み: {len(rows)} 行 / {len(races)} レース ({args.tsv})\n")

    all_win, all_ywin, all_imp = [], [], []
    fid_max = 0.0
    top1_model = top1_market = n_races = 0
    spear_model, spear_market, auc_model_races = [], [], []
    raw_var, prob_var = [], []
    q_rows = defaultdict(list)  # quarter -> [(model_win, y_win, implied)]
    gated_ids = set()  # 勝馬記録あり かつ オッズありのレース（(2)(3) も同一母数で測るため）

    for rid, rrows in races.items():
        win_p, _, _ = race_probs(rrows)
        fp = [_fp(r) for r in rrows]
        y_win = [1 if p == 1 else 0 for p in fp]
        implied, s = _implied(rrows)

        # 忠実性は鏡映の正しさ（outcome 非依存）なので全行で測る。
        for i, r in enumerate(rrows):
            fid_max = max(fid_max, abs(win_p[i] - float(r[COL["model_win"]])))

        # 以降の全指標は「勝馬記録あり かつ オッズあり」レースに母数を統一する（#272 レビュー）。
        # 結果欠損レース（全馬 y=0）を負例として混ぜず、model と market を同一レース集合で比較する。
        if not (any(y_win) and s > 0):
            continue
        gated_ids.add(rid)
        n_races += 1
        if y_win[argmax(win_p)] == 1:
            top1_model += 1
        if y_win[argmax(implied)] == 1:
            top1_market += 1
        spear_model.append(spearman(win_p, [-p for p in fp]))
        spear_market.append(spearman(implied, [-p for p in fp]))
        auc_model_races.append(auc(win_p, y_win))
        raw_var.append(pvar([raw_score(r, "win") for r in rrows]))
        prob_var.append(pvar(win_p))
        for i, r in enumerate(rrows):
            all_win.append(win_p[i]); all_ywin.append(y_win[i]); all_imp.append(implied[i])
            q_rows[quarter(r[COL["date"]])].append((win_p[i], y_win[i], implied[i]))

    if n_races == 0:
        print("勝馬記録のあるレースが 0 件です。入力 TSV を確認してください。", file=sys.stderr)
        return

    print("## (0) 忠実性アンカー（Python 再構成 ≡ Rust model_win）")
    ok = fid_max < 1e-6
    print(f"  max|python_win - dump_model_win| = {fid_max:.2e}  -> "
          f"{'OK' if ok else 'FAIL（再構成が不一致・以降の数値は無効）'}\n")
    if not ok:
        # 鏡映が前提なので、FAIL 時は無効な数値を並べず打ち切る（docstring の宣言と整合）。
        print("  忠実性 FAIL。定数/手順の鏡映を直すまで結論を出さない（以降の診断を打ち切り）。",
              file=sys.stderr)
        return

    print("## (1) resolution（純モデル vs 市場・勝馬記録ありかつオッズありのレースに統一）")
    print(f"  top1 的中率   model={top1_model/n_races:.3f}  "
          f"market={top1_market/n_races:.3f}  (n={n_races} レース)")
    print(f"  Spearman(race内, 確率 vs 着順)  model={nanmean(spear_model):.3f}  market={nanmean(spear_market):.3f}")
    print(f"  AUC(win, 全馬)  model={auc(all_win, all_ywin):.3f}  market={auc(all_imp, all_ywin):.3f}")
    print(f"  AUC(win, race内平均)  model={nanmean(auc_model_races):.3f}")
    print(f"  Brier(win)  model={brier(all_win, all_ywin):.4f}  market={brier(all_imp, all_ywin):.4f}")
    print(f"  LogLoss(win) model={logloss(all_win, all_ywin):.4f}  market={logloss(all_imp, all_ywin):.4f}\n")

    print("## (1b) resolution の窓別安定性（四半期）")
    print(f"  {'四半期':8} {'races':>6} {'top1_model':>11} {'top1_market':>12} {'AUC_model':>10} {'AUC_market':>11}")
    by_q = defaultdict(lambda: {"t1m": 0, "t1k": 0, "n": 0, "w": [], "i": [], "y": []})
    for rid, rrows in races.items():
        win_p, _, _ = race_probs(rrows)
        fp = [_fp(r) for r in rrows]
        y_win = [1 if p == 1 else 0 for p in fp]
        implied, s = _implied(rrows)
        if not (any(y_win) and s > 0):  # (1) と同一母数（勝馬記録あり かつ オッズあり）
            continue
        q = quarter(rrows[0][COL["date"]])
        b = by_q[q]
        b["n"] += 1
        if y_win[argmax(win_p)] == 1:
            b["t1m"] += 1
        if y_win[argmax(implied)] == 1:
            b["t1k"] += 1
        b["w"].extend(win_p); b["i"].extend(implied); b["y"].extend(y_win)
    for q in sorted(by_q):
        b = by_q[q]
        print(f"  {q:8} {b['n']:>6} {b['t1m']/b['n']:>11.3f} {b['t1k']/b['n']:>12.3f} "
              f"{auc(b['w'], b['y']):>10.3f} {auc(b['i'], b['y']):>11.3f}")
    print()

    # 欠落率は全ダンプ行（データ可用性）で測り、レース内分散・相関は (1) と同一母数（gated）で測る。
    # corr は show レートと複勝（1<=着順<=3）の代理指標であり、win スコアそのものの識別力ではない点に注意。
    print("## (2) 素性別 識別力・欠落率（分散/相関は勝馬+オッズありレース・corr は show率代理）")
    print(f"  {'factor':24} {'欠落率':>7} {'race内分散(平均)':>16} {'corr(show率,複勝)':>18}")
    n_tot = len(rows)
    for name, base, w in STAT_FACTORS:
        miss = sum(1 for r in rows if _stat_cell(r, base) is None)  # 欠落率は全行
        within_var, rates, ys = [], [], []
        for rid, rrows in races.items():
            if rid not in gated_ids:
                continue
            vals = []
            for r in rrows:
                fs = _stat_cell(r, base)
                if fs is None:
                    continue
                sh = shrink_rate(fs[2], fs[3], PRIOR["show"])
                vals.append(sh); rates.append(sh)
                ys.append(1 if 1 <= _fp(r) <= 3 else 0)
            if len(vals) >= 2:
                within_var.append(pvar(vals))
        print(f"  {name:24} {miss/n_tot:>7.3f} {mean(within_var):>16.5f} {corr(rates, ys):>18.3f}")
    for name, idx, w in SCALAR_FACTORS:
        miss = sum(1 for r in rows if r[idx] == "")
        print(f"  {name:24} {miss/n_tot:>7.3f} {'(scalar w=%.2f)' % w:>16} {'':>18}")
    print()

    print("## (3) leave-one-out ablation（外して悪化＝有用。改善＝害）")
    base_brier = brier(all_win, all_ywin)
    base_ll = logloss(all_win, all_ywin)
    base_top1 = top1_model / n_races
    print(f"  baseline  Brier={base_brier:.4f}  LogLoss={base_ll:.4f}  top1={base_top1:.3f}")
    drops = [n for n, _, _ in STAT_FACTORS] + [n for n, _, w in SCALAR_FACTORS if w > 0]
    for name in drops:
        dw, dy, dtop1, dn = [], [], 0, 0
        for rid, rrows in races.items():
            if rid not in gated_ids:  # baseline（all_win/all_ywin）と同一母数で比較する
                continue
            wp, _, _ = race_probs(rrows, drop=name)
            yw = [1 if _fp(r) == 1 else 0 for r in rrows]
            dn += 1
            if yw[argmax(wp)] == 1:
                dtop1 += 1
            dw.extend(wp); dy.extend(yw)
        print(f"  -{name:22} ΔBrier={brier(dw,dy)-base_brier:+.4f}  "
              f"ΔLogLoss={logloss(dw,dy)-base_ll:+.4f}  Δtop1={dtop1/dn-base_top1:+.3f}")
    print()

    print("## (4) 正規化の圧縮度（raw_score の分散が prob でどれだけ潰れるか）")
    mr, mp = mean(raw_var), mean(prob_var)
    print(f"  mean var(raw_score_win, race内) = {mr:.5f}")
    print(f"  mean var(model_win,   race内)  = {mp:.5f}")
    print(f"  圧縮比 prob/raw = {mp/max(mr,1e-12):.3f}\n")

    print("## (5) isotonic 上限効果（walk-forward 前窓 fit→後窓適用・リーク無し）")
    quarters = sorted(q_rows.keys())
    iso_w, base_w, mkt_w, base_y = [], [], [], []
    for k in range(1, len(quarters)):
        train = q_rows[quarters[k - 1]]
        test = q_rows[quarters[k]]
        thr, fit = pava_fit([t[0] for t in train], [t[1] for t in train])
        for w, y, imp in test:
            iso_w.append(pava_apply(thr, fit, w))
            base_w.append(w); mkt_w.append(imp); base_y.append(y)
    if base_y:
        bb, bi, bm = brier(base_w, base_y), brier(iso_w, base_y), brier(mkt_w, base_y)
        print(f"  Brier(win)  pure={bb:.4f}  pure+isotonic={bi:.4f}  market={bm:.4f}  (窓 {len(quarters)})")
        gap = bb - bm
        closed = (bb - bi) / gap if abs(gap) > 1e-9 else float("nan")
        print(f"  isotonic が市場との gap を詰めた割合 = {closed:.1%}")
        verdict = ("calibration 限定（isotonic で市場≈に到達）" if bi <= bm * 1.02
                   else "resolution 限定（isotonic でも市場に届かず＝素性改善が必要）")
        print(f"  -> {verdict}")
    print()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--tsv", required=True, help="純モデル(α=1.0) の --dump-features TSV")
    run(ap.parse_args())


if __name__ == "__main__":
    main()
