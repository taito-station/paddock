#!/usr/bin/env python3
"""市場非依存の独立確率モデル（#720）: 上位 k 着の割引 Plackett-Luce。

各馬の効用 u = β·x（x は市場情報を含まない特徴量）から、1 着段は softmax(u)、s 着段（s≥2）は
残っている馬の softmax(λ_s·u) で着順が決まるとみなし、上位 k 着の尤度を最大化する（L2 正則化付き）。
λ1 = 1 で固定（識別性）。3 着段以降は λ3 を共有する。

この形は評価器 `scripts/predict-check/prob_ledger.py` の割引 Harville と同じ:
p = softmax(u) とすると exp(λ·u) ∝ p^λ なので、段 s の条件付き確率 p_j^λ / Σ_残 p^λ に一致する。
そのため確率 TSV には p_win（= softmax(u)）と、その予測に使ったモデルの λ2・λ3 を出せば、
prob_ledger が馬連・ワイド・3連複・連対の確率を同じモデルのまま再構成できる。

着順の規約（stage_winners）:
- 1 着から順に、その着順の馬がちょうど 1 頭いる間だけ段を作る。同着や欠落の位置の手前で打ち切る。
- 着順の無い馬（中止など）は分母（残っている馬）には入り続け、選ばれる側には入らない。

識別できない λ は 1.0 に固定する（k=1 なら λ2=λ3=1、k=2 なら λ3=1）。

walk-forward（CLI）: 予測月ごとに、学習開始日〜月初の前日のレースで標準化と学習をやり直し、その月の
レースの p_win（全桁）と、そのモデルの λ を `race_id horse_num p_win lam2 lam3` の TSV に出す。
特徴量 TSV は pl_features.py の出力（各レースの特徴量は as-of で一度だけ決まっている）。

使い方:
  python3 scripts/harness/train_pl_topk.py features.tsv -o probs.tsv --k 3 \\
      --train-from 2025-03-01 --pred-from 2025-07-01 --pred-to 2025-12-31 --meta meta.json
  （--fix-lambda で λ=1 の素の PL top-k。prob_ledger で割引 λ の効き目を対応ありで比べる）
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
from dataclasses import dataclass

import numpy as np
from scipy.optimize import minimize

import pl_features

LAM_BOUNDS = (0.05, 5.0)
# 予測月ごとの学習に要る最小レース数（段を作れるレース）。未満の月は予測を出さずに理由を残す
MIN_TRAIN_RACES = 50


def stage_winners(fins: list, k: int) -> list[int]:
    """1〜k 着のうち、一意に決まる先頭からの着順の行インデックス（同着・欠落の手前で打ち切る）。"""
    out = []
    for pos in range(1, k + 1):
        hits = [i for i, f in enumerate(fins) if f == pos]
        if len(hits) != 1:
            break
        out.append(hits[0])
    return out


def pack(races: list[tuple[np.ndarray, list[int]]], n_feat: int) -> dict:
    """[(特徴量 (n_i, F), 段の勝者)] をパディングした配列にまとめる。

    X: (R, H, F)・valid: (R, H) 実在の馬・win: (R, S) 段の勝者の行（無い段は -1）。
    """
    r = len(races)
    h = max((x.shape[0] for x, _ in races), default=1)
    s = max((len(w) for _, w in races), default=0)
    x_all = np.zeros((r, h, n_feat))
    valid = np.zeros((r, h), dtype=bool)
    win = -np.ones((r, max(s, 1)), dtype=int)
    for i, (x, w) in enumerate(races):
        n = x.shape[0]
        x_all[i, :n] = x
        valid[i, :n] = True
        win[i, : len(w)] = w
    return {"X": x_all, "valid": valid, "win": win}


def _stage_lams(k: int, lam2: float, lam3: float) -> list[float]:
    return [1.0] + [lam2] + [lam3] * max(0, k - 2)


def neg_log_lik(theta: np.ndarray, data: dict, k: int, l2: float) -> tuple[float, np.ndarray]:
    """θ = [β(F), λ2, λ3] の負の対数尤度と勾配。k=1 では λ、k=2 では λ3 の勾配は 0（使われない）。"""
    x, valid, win = data["X"], data["valid"], data["win"]
    n_feat = x.shape[2]
    beta, lam2, lam3 = theta[:n_feat], float(theta[n_feat]), float(theta[n_feat + 1])
    u = x @ beta  # (R, H)
    avail = valid.copy()
    nll = 0.5 * l2 * float(beta @ beta)
    g_beta = l2 * beta.copy()
    g_lam = np.zeros(2)
    rows_all = np.arange(x.shape[0])
    for s, lam in enumerate(_stage_lams(k, lam2, lam3)):
        if s >= win.shape[1]:
            break
        has = win[:, s] >= 0
        if not has.any():
            break
        rows = rows_all[has]
        w = win[has, s]
        logits = np.where(avail[rows], lam * u[rows], -np.inf)
        m = logits.max(axis=1, keepdims=True)
        e = np.exp(logits - m)
        z = e.sum(axis=1, keepdims=True)
        p = e / z  # (n, H)
        lse = (m + np.log(z))[:, 0]
        u_w = u[rows, w]
        nll -= float((lam * u_w - lse).sum())
        x_w = x[rows, w]  # (n, F)
        ex = np.einsum("nh,nhf->nf", p, x[rows])
        g_beta -= lam * (x_w - ex).sum(axis=0)
        if s >= 1:
            eu = (p * np.where(avail[rows], u[rows], 0.0)).sum(axis=1)
            g_lam[0 if s == 1 else 1] -= float((u_w - eu).sum())
        avail[rows, w] = False
    return nll, np.concatenate([g_beta, g_lam])


@dataclass
class Model:
    beta: np.ndarray
    lam2: float = 1.0
    lam3: float = 1.0


def fit(data: dict, k: int, l2: float, beta0: np.ndarray | None = None, fix_lambda: bool = False) -> Model:
    """L-BFGS-B で β と（識別できる）λ を推定する。fix_lambda なら λ=1（素の PL）。収束しなければ RuntimeError。"""
    n_feat = data["X"].shape[2]
    theta0 = np.concatenate([beta0 if beta0 is not None else np.zeros(n_feat), [1.0, 1.0]])
    bounds = [(None, None)] * n_feat
    bounds.append(LAM_BOUNDS if k >= 2 and not fix_lambda else (1.0, 1.0))
    bounds.append(LAM_BOUNDS if k >= 3 and not fix_lambda else (1.0, 1.0))
    res = minimize(neg_log_lik, theta0, args=(data, k, l2), jac=True, method="L-BFGS-B", bounds=bounds)
    if not res.success:
        raise RuntimeError(f"PL の最適化が収束しなかった: {res.message}")
    return Model(beta=res.x[:n_feat], lam2=float(res.x[n_feat]), lam3=float(res.x[n_feat + 1]))


def predict_race(model: Model, x: np.ndarray) -> np.ndarray:
    """1 レースの単勝確率 softmax(β·x)。後処理（較正・混合）はしない（λ との対応が崩れるため）。"""
    u = x @ model.beta
    e = np.exp(u - u.max())
    return e / e.sum()


# ---------- walk-forward ----------


@dataclass
class Race:
    date: str
    horse_nums: list
    x: np.ndarray  # (n, F) 生の特徴量
    fins: list  # 着順（中止は None）


def load_features(path: str, features: list[str] | None = None) -> tuple[dict, list[str]]:
    """pl_features.py の出力を {race_id: Race} で読む。

    使う列は既定で pl_features.FEATURES（市場非依存の許可リスト）。features で絞れるが、許可リスト外の列は
    指定できない（後から TSV に結合した列を黙って学習に使わない）。値は有限、(race_id, horse_num) は一意、
    レース内の date は一致していること。
    """
    cols = list(features or pl_features.FEATURES)
    outside = [c for c in cols if c not in pl_features.FEATURES]
    if outside:
        raise ValueError(f"{path}: 特徴量の許可リスト（pl_features.FEATURES）にない列です: {outside}")
    races: dict = {}
    with open(path, encoding="utf-8", newline="") as f:
        reader = csv.DictReader(f, delimiter="\t")
        missing = [c for c in cols if c not in reader.fieldnames]
        if missing:
            raise ValueError(f"{path}: 特徴量列がありません: {missing}")
        buf: dict = {}
        seen = set()
        for lineno, r in enumerate(reader, start=2):
            key = (r["race_id"], r["horse_num"])
            if key in seen:
                raise ValueError(f"{path}:{lineno} 重複行 {key}")
            seen.add(key)
            vals = [float(r[c]) for c in cols]
            if not all(np.isfinite(vals)):
                raise ValueError(f"{path}:{lineno} 特徴量に有限でない値があります")
            rows = buf.setdefault(r["race_id"], [])
            if rows and rows[0]["date"] != r["date"]:
                raise ValueError(f"{path}:{lineno} {r['race_id']} の date がレース内で一致しません")
            r["_x"] = vals
            rows.append(r)
    for rid, rows in buf.items():
        races[rid] = Race(
            date=rows[0]["date"],
            horse_nums=[int(r["horse_num"]) for r in rows],
            x=np.array([r["_x"] for r in rows]),
            fins=[int(r["finishing_position"]) if r["finishing_position"] else None for r in rows],
        )
    return races, cols


def fit_scaler(x: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    mu = x.mean(axis=0)
    sd = x.std(axis=0)
    sd[sd < 1e-9] = 1.0  # 定数列は割らない（学習窓で動かない列）
    return mu, sd


def apply_scaler(x: np.ndarray, mu: np.ndarray, sd: np.ndarray) -> np.ndarray:
    return (x - mu) / sd


def _months(frm: str, to: str) -> list[str]:
    y, m = int(frm[:4]), int(frm[5:7])
    out = []
    while f"{y:04d}-{m:02d}" <= to[:7]:
        out.append(f"{y:04d}-{m:02d}")
        y, m = (y + 1, 1) if m == 12 else (y, m + 1)
    return out


def walk_forward(
    races: dict, train_from: str, pred_from: str, pred_to: str, k: int, l2: float, fix_lambda: bool = False,
    min_train: int = MIN_TRAIN_RACES,
) -> tuple[list[dict], list[dict]]:
    """予測月ごとに学習し直して確率を出す。戻り値は (出力行, 月ごとのモデル情報)。

    学習レース（段を作れるもの）が min_train 未満の月は予測を出さず、meta に skipped（理由）を残す。
    最適化が収束しなければ、その月を示して RuntimeError にする。
    """
    out, meta = [], []
    for month in _months(pred_from, pred_to):
        start = month + "-01"
        train = [r for r in races.values() if train_from <= r.date < start]
        pred = {rid: r for rid, r in races.items() if r.date[:7] == month and pred_from <= r.date <= pred_to}
        if not pred:
            continue
        # 段の作れないレース（1 着同着・結果なし）は学習に寄与しない
        usable = [r for r in train if stage_winners(r.fins, k)]
        if len(usable) < min_train:
            meta.append({
                "month": month, "n_train_races": len(usable), "n_pred_races": len(pred),
                "skipped": f"学習レース {len(usable)} < {min_train}（{train_from}〜{start} の前日）",
            })
            continue
        # 標準化は学習窓の全レース（段を作れないレースも特徴量の分布には含める）
        mu, sd = fit_scaler(np.concatenate([r.x for r in train]))
        packed = [(apply_scaler(r.x, mu, sd), stage_winners(r.fins, k)) for r in usable]
        try:
            model = fit(pack(packed, len(mu)), k, l2, fix_lambda=fix_lambda)
        except RuntimeError as e:
            raise RuntimeError(f"{month}: {e}") from e
        for rid in sorted(pred):
            r = pred[rid]
            p = predict_race(model, apply_scaler(r.x, mu, sd))
            for h, pw in zip(r.horse_nums, p):
                out.append({"race_id": rid, "horse_num": h, "p_win": float(pw), "lam2": model.lam2, "lam3": model.lam3})
        meta.append({
            "month": month, "n_train_races": len(packed), "n_pred_races": len(pred),
            "lam2": model.lam2, "lam3": model.lam3, "beta": [float(b) for b in model.beta],
        })
    return out, meta


def write_probs(path: str, rows: list[dict]) -> None:
    with open(path, "w", encoding="utf-8", newline="") as f:
        w = csv.writer(f, delimiter="\t", lineterminator="\n")
        w.writerow(["race_id", "horse_num", "p_win", "lam2", "lam3"])
        for r in rows:
            w.writerow([r["race_id"], r["horse_num"], repr(r["p_win"]), repr(r["lam2"]), repr(r["lam3"])])


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("features", help="pl_features.py の出力 TSV")
    ap.add_argument("-o", "--out", required=True, help="確率 TSV（prob_ledger の --system に渡す）")
    ap.add_argument("--k", type=int, default=3, choices=range(1, 6), help="尤度に使う上位着数（1〜5）")
    ap.add_argument("--l2", type=float, default=1.0,
                    help="β の L2 係数（レース数で正規化しない総和の尤度に足す。学習窓が長いほど実効的に弱まる）")
    ap.add_argument("--min-train-races", type=int, default=MIN_TRAIN_RACES)
    ap.add_argument("--train-from", default="2025-03-01")
    ap.add_argument("--pred-from", default="2025-07-01")
    ap.add_argument("--pred-to", default="2025-12-31")
    ap.add_argument("--fix-lambda", action="store_true", help="λ=1 に固定（素の PL top-k）")
    ap.add_argument("--cols", help="使う特徴量列（カンマ区切り・既定は pl_features.FEATURES。許可リスト外は不可）")
    ap.add_argument("--meta", help="月ごとのモデル情報（JSON）の出力先")
    args = ap.parse_args(argv)
    races, cols = load_features(args.features, args.cols.split(",") if args.cols else None)
    out, meta = walk_forward(races, args.train_from, args.pred_from, args.pred_to, args.k, args.l2, args.fix_lambda,
                             args.min_train_races)
    write_probs(args.out, out)
    for m in meta:
        if "skipped" in m:
            print(f"{m['month']}: 予測なし（{m['skipped']}）", file=sys.stderr)
            continue
        print(f"{m['month']}: 学習 {m['n_train_races']}R / 予測 {m['n_pred_races']}R / λ={m['lam2']:.3f},{m['lam3']:.3f}",
              file=sys.stderr)
    if args.meta:
        with open(args.meta, "w", encoding="utf-8") as f:
            json.dump({"args": vars(args), "features": cols, "months": meta}, f, ensure_ascii=False, indent=1)
    return 0


if __name__ == "__main__":
    sys.exit(main())
