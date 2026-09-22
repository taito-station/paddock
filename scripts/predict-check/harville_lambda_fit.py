#!/usr/bin/env python3
"""discounted Harville（Lo & Bacon-Shone）の λ2/λ3 を dump TSV から推定・検証する（#703 Phase 2）。

やること:
  (1) 導入前バケット検証（Benter 1994 Table 9/10 型・「実測してから直す」規律）:
      λ=1（素の Harville）の条件付き 2 着確率 P(2nd=i|1st=w) = p_i/(1−p_w) をバケットに切り、
      実測 2 着率と二項 Z 検定で突合する。3 着段も同様。高確率帯で Z<0・低確率帯で Z>0 なら
      「強い馬の 2・3 着を過大評価」（Benter の観測と同方向）。
  (2) λ の MLE（fit 窓のみ）:
      対数尤度が着順位置ごとに分離する（Lo & Bacon-Shone）ため、
        λ2: Σ_races ln[ p_2nd^λ2 / Σ_{j≠1着} p_j^λ2 ]
        λ3: Σ_races ln[ p_3rd^λ3 / Σ_{j∉{1,2着}} p_j^λ3 ]
      を独立に 1 次元最適化（黄金分割）。CI はプロファイル尤度（ΔlogL = 1.92 ≈ 95%）。
  (3) eval 窓検証: fit 窓の λ̂ を当てて eval 窓の logL 改善とバケット Z の縮小を確認する。

確率系統は --probs pure|blended で選ぶ。**pure と blended で最適 λ は別物**（市場の
favorite-longshot バイアスが Harville バイアスを部分相殺する・Benter Note 3）。
事前値の目安: 伊藤 2010（JRA 44,774R）λ2=0.81/λ3=0.70、Benter（香港）0.81/0.65。

母集合: 1・2・3 着がそれぞれ一意に取れるレースのみ（同着・着順欠落は除外し件数を出す。
勝者の着順が PDF から取れないレースは prob_eval と同じく除外される）。

使い方:
  python3 scripts/predict-check/harville_lambda_fit.py bt_dump.tsv \
      --probs pure --fit-until 2025-12-31 --eval-from 2026-01-01
"""

from __future__ import annotations

import argparse
from collections import OrderedDict

import numpy as np

import prob_eval as pe

# λ の探索範囲。伊藤 2010 / Benter の実測（0.6〜0.85）を大きく包み、pure 系統で観測された
# 逆方向（λ>1 = 純モデルの条件付き段が平坦すぎる）も上限に張り付かず推定できる幅を取る。
_LAMBDA_LO, _LAMBDA_HI = 0.2, 6.0


def podium_races(
    races: "OrderedDict[str, list[dict]]",
) -> tuple["OrderedDict[str, tuple[list[dict], int, int, int]]", dict]:
    """1・2・3 着が一意なレースだけを (rows, idx1, idx2, idx3) で返す。除外件数も返す。"""
    out: "OrderedDict[str, tuple[list[dict], int, int, int]]" = OrderedDict()
    excluded = {"no_unique_podium": 0}
    for rid, rows in races.items():
        idx = {}
        ok = True
        for pos in (1, 2, 3):
            hits = [i for i, r in enumerate(rows) if r.get("finishing_position") == pos]
            if len(hits) != 1:
                ok = False
                break
            idx[pos] = hits[0]
        if ok:
            out[rid] = (rows, idx[1], idx[2], idx[3])
        else:
            excluded["no_unique_podium"] += 1
    return out, excluded


def _probs(rows: list[dict], key: str) -> np.ndarray:
    """行から確率列を取り出す。欠落（None）・値域外（p<=0 or p>1）は λ̂/CI/Z を黙って歪める
    ため即エラーにする（load_dump の「静かな汚染より停止」方針に揃える）。"""
    out = []
    for r in rows:
        v = r.get(key)
        if v is None:
            raise ValueError(f"race {r.get('race_id')!r}: 列 {key!r} が空です（旧 dump の可能性。再生成してください）")
        v = float(v)
        if not (0.0 < v <= 1.0):
            raise ValueError(f"race {r.get('race_id')!r}: 列 {key!r} の値 {v} が確率の値域 (0,1] 外です")
        out.append(v)
    return np.array(out)


def stage_loglik(
    races: "OrderedDict[str, tuple[list[dict], int, int, int]]",
    key: str,
    stage: int,
    lam: float,
) -> float:
    """位置 stage（2 or 3）の条件付き多項対数尤度 Σ ln[ p_winner^λ / Σ_残 p^λ ]。"""
    total = 0.0
    for rows, i1, i2, i3 in races.values():
        p = _probs(rows, key)
        if stage == 2:
            mask = np.ones(len(p), dtype=bool)
            mask[i1] = False
            target = i2
        else:
            mask = np.ones(len(p), dtype=bool)
            mask[i1] = False
            mask[i2] = False
            target = i3
        pl = np.power(np.maximum(p[mask], 1e-12), lam)
        denom = pl.sum()
        # target の mask 内位置
        pt = max(p[target], 1e-12) ** lam
        total += float(np.log(pt / denom))
    return total


def fit_lambda(
    races: "OrderedDict[str, tuple[list[dict], int, int, int]]", key: str, stage: int
) -> tuple[float, tuple[float, float], float]:
    """黄金分割で λ̂ を求め、プロファイル尤度 CI（ΔlogL=1.92）と対 λ=1 の logL 改善を返す。"""
    gr = (np.sqrt(5.0) - 1.0) / 2.0
    a, b = _LAMBDA_LO, _LAMBDA_HI
    c, d = b - gr * (b - a), a + gr * (b - a)
    fc, fd = stage_loglik(races, key, stage, c), stage_loglik(races, key, stage, d)
    for _ in range(60):
        if fc > fd:
            b, d, fd = d, c, fc
            c = b - gr * (b - a)
            fc = stage_loglik(races, key, stage, c)
        else:
            a, c, fc = c, d, fd
            d = a + gr * (b - a)
            fd = stage_loglik(races, key, stage, d)
    lam_hat = (a + b) / 2.0
    if min(lam_hat - _LAMBDA_LO, _LAMBDA_HI - lam_hat) < 0.05:
        print(
            f"警告: λ̂={lam_hat:.3f} が探索境界 [{_LAMBDA_LO}, {_LAMBDA_HI}] に接近。"
            "真の最適値は域外の可能性があるため範囲を広げて再実行を推奨。"
        )
    ll_hat = stage_loglik(races, key, stage, lam_hat)
    # プロファイル CI: グリッド走査で ll >= ll_hat - 1.92 の範囲。
    grid = np.linspace(_LAMBDA_LO, _LAMBDA_HI, 181)
    lls = np.array([stage_loglik(races, key, stage, g) for g in grid])
    inside = grid[lls >= ll_hat - 1.92]
    ci = (float(inside.min()), float(inside.max())) if len(inside) else (lam_hat, lam_hat)
    improvement = ll_hat - stage_loglik(races, key, stage, 1.0)
    return float(lam_hat), ci, float(improvement)


def bucket_z_table(
    races: "OrderedDict[str, tuple[list[dict], int, int, int]]",
    key: str,
    stage: int,
    lam: float,
    n_buckets: int = 8,
) -> list[dict]:
    """条件付き 2/3 着確率のバケット別 予測 vs 実測（二項 Z）。Benter Table 9/10 型。

    各レースの「残り馬全頭」について λ 割引の条件付き確率を予測とし、その馬が実際に
    当該着順だったかを実測とする。バケットは予測確率の分位（等度数）。

    Z の分散は独立二項近似（Var=Σp(1−p)）で、同一レース内の one-hot 制約による負相関を
    無視している。真の分散はこれより小さく |Z| は過小＝**保守側**。方向判定には十分だが
    Z の絶対値を厳密な有意性として読まないこと（Benter Table 9/10 と同じ慣行）。
    """
    preds: list[float] = []
    hits: list[float] = []
    for rows, i1, i2, i3 in races.values():
        p = _probs(rows, key)
        if stage == 2:
            rest = [i for i in range(len(p)) if i != i1]
            actual = i2
        else:
            rest = [i for i in range(len(p)) if i not in (i1, i2)]
            actual = i3
        pl = np.power(np.maximum(p[rest], 1e-12), lam)
        cond = pl / pl.sum()
        for i, q in zip(rest, cond):
            preds.append(float(q))
            hits.append(1.0 if i == actual else 0.0)
    preds_a = np.array(preds)
    hits_a = np.array(hits)
    edges = np.quantile(preds_a, np.linspace(0, 1, n_buckets + 1))
    out = []
    for k in range(n_buckets):
        lo, hi = edges[k], edges[k + 1]
        sel = (
            (preds_a >= lo) & (preds_a <= hi)
            if k == n_buckets - 1
            else (preds_a >= lo) & (preds_a < hi)
        )
        n = int(sel.sum())
        if n == 0:
            continue
        exp = float(preds_a[sel].sum())
        obs = float(hits_a[sel].sum())
        var = float((preds_a[sel] * (1 - preds_a[sel])).sum())
        z = (obs - exp) / np.sqrt(var) if var > 0 else 0.0
        out.append(
            {
                "range": (float(lo), float(hi)),
                "n": n,
                "mean_pred": exp / n,
                "obs_rate": obs / n,
                "z": float(z),
            }
        )
    return out


def _print_bucket_table(title: str, table: list[dict]) -> None:
    print(f"  {title}")
    print(f"    {'予測帯':>19} {'n':>7} {'平均予測':>8} {'実測率':>8} {'Z':>7}")
    for b in table:
        lo, hi = b["range"]
        print(
            f"    [{lo:7.4f},{hi:7.4f}] {b['n']:>7} {b['mean_pred']:>8.4f}"
            f" {b['obs_rate']:>8.4f} {b['z']:>+7.2f}"
        )


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("dump", help="analyze backtest --dump-features の TSV（56 列版）")
    ap.add_argument(
        "--probs",
        choices=["pure", "blended"],
        required=True,
        help="λ を当てる確率系統（pure=model_win_pure / blended=model_win）",
    )
    ap.add_argument("--fit-until", required=True, help="fit 窓の終端日（YYYY-MM-DD・含む）")
    ap.add_argument("--eval-from", required=True, help="eval 窓の開始日（YYYY-MM-DD・含む）")
    args = ap.parse_args()

    err = pe.validate_windows(args.fit_until, args.eval_from)
    if err:
        ap.error(err)
    key = "model_win_pure" if args.probs == "pure" else "model_win"

    rows = pe.load_dump(args.dump)
    fit_rows, eval_rows = pe.split_window(rows, args.fit_until, args.eval_from)

    print(f"系統: {args.probs}（列 {key}）")
    results = {}
    for name, wrows in (("fit", fit_rows), ("eval", eval_rows)):
        table = pe.build_race_table(wrows)
        usable = table.winner_races()
        podium, excluded = podium_races(usable)
        print(
            f"\n== {name} 窓: レース {len(table.races)} → 表彰台一意 {len(podium)}"
            f"（勝者非一意除外 {table.excluded['dead_heat'] + table.excluded['no_winner']}"
            f" / 2・3 着非一意除外 {excluded['no_unique_podium']}） =="
        )
        results[name] = podium

    fit_races = results["fit"]
    eval_races = results["eval"]
    # 空窓は黙って任意の λ̂（logL 恒等 0 で探索域中央付近）を出してしまうため明示エラー。
    for name, races in (("fit", fit_races), ("eval", eval_races)):
        if not races:
            ap.error(f"{name} 窓に表彰台一意のレースが 0 件です。窓指定と dump を確認してください")

    print("\n-- (1) 導入前バケット検証（λ=1・fit 窓） --")
    for stage in (2, 3):
        _print_bucket_table(
            f"{stage} 着段（高確率帯で Z<0 なら過大評価 = Benter と同方向）",
            bucket_z_table(fit_races, key, stage, 1.0),
        )

    print("\n-- (2) λ の MLE（fit 窓のみ） --")
    lam = {}
    for stage in (2, 3):
        lam_hat, ci, imp = fit_lambda(fit_races, key, stage)
        lam[stage] = lam_hat
        print(
            f"  λ{stage} = {lam_hat:.3f}  [95% CI {ci[0]:.3f}, {ci[1]:.3f}]"
            f"  logL 改善（対 λ=1）= {imp:+.1f}"
        )
    print("  事前値の目安: 伊藤 2010（JRA）λ2=0.81/λ3=0.70・Benter（香港）0.81/0.65")

    print("\n-- (3) eval 窓検証（fit 窓の λ̂ を適用） --")
    for stage in (2, 3):
        imp_eval = stage_loglik(eval_races, key, stage, lam[stage]) - stage_loglik(
            eval_races, key, stage, 1.0
        )
        print(f"  λ{stage}={lam[stage]:.3f}: eval logL 改善（対 λ=1）= {imp_eval:+.1f}")
        _print_bucket_table(
            f"{stage} 着段バケット（補正後・eval 窓）",
            bucket_z_table(eval_races, key, stage, lam[stage]),
        )
        _print_bucket_table(
            f"{stage} 着段バケット（λ=1・eval 窓・比較用）",
            bucket_z_table(eval_races, key, stage, 1.0),
        )


if __name__ == "__main__":
    main()
