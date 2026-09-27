#!/usr/bin/env python3
"""独立確率の評価器（#719）: 系統ごとの 馬連・連対・ワイド・3連複・単勝 の確率を dump の着順で評価し、
版ごとの ledger に追記する。

`prob_eval.py`（#703 の採否ゲート・単勝中心・系統固定）とは別に、**任意の確率系統**（外部 TSV）を
差し込み、券種の優先度（馬連 > ワイド > 3連複 > 単勝）どおりに組合せ確率の当てはまりを測る。
比較の基準（baseline）は dump の純モデル `model_win_pure` → 割引 Harville。市場（単勝オッズ）は
参考列として別の母集合で並べるだけで、ゲートにはしない。

組合せ確率（割引 Harville / Lo & Bacon-Shone・harville_lambda_fit.py と同じ段別定義）:
  P(i が 1 着) = p_i / Σp
  P(j が 2 着 | i) = p_j^λ2 / Σ_{m≠i} p_m^λ2
  P(k が 3 着 | i,j) = p_k^λ3 / Σ_{m∉{i,j}} p_m^λ3
  馬連 = 1-2 着の順不同ペア、連対 = 馬単位 top2、ワイド = 2 頭とも 3 着以内、3連複 = 3 頭の順不同組。
  λ2=λ3=1 が素の Harville（本番 pure の RECOMMENDED_HARVILLE_LAMBDA_PURE = IDENTITY）。

指標（レース単位。擬似 R² は一様予測比 = 1 − Σ NLL / Σ NLL_一様）:
  主: 馬連 NLL = −ln P(実際の 1-2 着ペア)、連対 = 馬単位の二値 log-loss
  副: ワイド NLL = 的中 3 ペアの −ln P の平均、3連複 NLL = −ln P(実際の 3 頭)、単勝 NLL、
      Brier（連対・馬連ペア・ワイドペア）、CORP 分解（連対・ワイドペア）
  95% CI はレース単位のブートストラップ（系統間の差は同じ再標本を使う対応あり）。

母集合（券種別）: 単勝 = 1 着が一意 / 馬連・連対 = 1・2 着が一意 / ワイド・3連複 = 1〜3 着が一意。
着順なしの馬（中止など）は、表彰台が確定していればラベル 0 として残す（prob_eval._label と同じ）。
系統間は「全系統が全出走馬に確率を持つ」レースに揃える（欠けたレースは理由別に件数を出す）。

窓: dev（既定 2025-07-01〜2025-12-31・#703 fit 窓の内側）は反復・調整用で採否根拠にしない。
test（既定 2026-01-01〜2026-08-31 = #703 eval 窓）は版の節目にだけ測る（`--windows test`）。

外部確率 TSV: ヘッダ `race_id  horse_num  p_win` ＋任意の `p_top2`（連対を直接出す系統はこれを優先）。

使い方:
  python3 scripts/predict-check/prob_ledger.py bt_dump_<sha>.tsv \\
      --system pl_topk=probs.tsv --lambda pl_topk=0.9,0.8 \\
      --ledger docs/docs-original/719-prob-ledger.md --label "v1 pl_topk k=3"
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import math
import os
import re
import subprocess
import sys
from collections import Counter, OrderedDict
from dataclasses import dataclass
from datetime import date as _date
from itertools import combinations, permutations

import numpy as np

import prob_eval as pe

FLOOR = pe._PROB_FLOOR
_ISO_DATE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
_HORSE_NUM = re.compile(r"^[1-9][0-9]*$")
# #703 で凍結した eval 窓（backtest.md「評価プロトコル」）。dev 窓がここに重なると、記録なしで eval 窓を覗ける
FROZEN_EVAL_FROM, FROZEN_EVAL_TO = "2026-01-01", "2026-08-31"


# ---------- 組合せ確率 ----------


@dataclass
class Combo:
    """1 レースの組合せ確率。行列はレース内の馬の並び（dump の行順）でインデックスする。"""

    win: np.ndarray  # (n,)
    top2: np.ndarray  # (n,) 連対
    quinella: np.ndarray  # (n,n) 対称・対角 0
    wide: np.ndarray  # (n,n) 対称・対角 0
    trio: np.ndarray  # (n,n,n) 完全対称・添字重複は 0
    _ordered: np.ndarray  # (n,n,n) 順序付き 1-2-3 着

    def ordered3(self, i: int, j: int, k: int) -> float:
        return float(self._ordered[i, j, k])


def _safe_div(num: np.ndarray, den: np.ndarray) -> np.ndarray:
    return np.divide(num, den, out=np.zeros(np.broadcast(num, den).shape), where=den > 0)


def combo_probs(p: np.ndarray, lam2: float = 1.0, lam3: float = 1.0) -> Combo:
    """単勝確率 p（未正規化可）から割引 Harville の組合せ確率を作る。"""
    p = np.asarray(p, dtype=float)
    if not np.all(np.isfinite(p)) or np.any(p < 0) or p.sum() <= 0:
        raise ValueError(f"単勝確率が不正です（非有限・負・和 0）: {p}")
    n = len(p)
    win = p / p.sum()
    w2 = np.power(win, lam2)
    w3 = np.power(win, lam3)
    eye = np.eye(n, dtype=bool)
    # exacta[i,j] = P(1着=i, 2着=j)
    den2 = w2.sum() - w2  # (n,) 1 着 i を除いた λ2 重みの和
    exacta = win[:, None] * _safe_div(w2[None, :], den2[:, None])
    exacta[eye] = 0.0
    # ordered[i,j,k] = exacta[i,j] * w3_k / (Σw3 − w3_i − w3_j)
    den3 = w3.sum() - w3[:, None] - w3[None, :]  # (n,n)
    ordered = exacta[:, :, None] * _safe_div(w3[None, None, :], den3[:, :, None])
    idx = np.arange(n)
    ordered[idx, :, idx] = 0.0  # k == i
    ordered[:, idx, idx] = 0.0  # k == j
    ordered[idx, idx, :] = 0.0  # j == i
    trio = sum(np.transpose(ordered, axes) for axes in permutations((0, 1, 2)))
    wide = trio.sum(axis=2)
    quinella = exacta + exacta.T
    top2 = win + exacta.sum(axis=0)
    return Combo(win=win, top2=top2, quinella=quinella, wide=wide, trio=trio, _ordered=ordered)


# ---------- 着順・レース単位スコア ----------


def podium(fins: list) -> dict:
    """1・2・3 着それぞれの行インデックス（一意でなければ None）。"""
    out = {}
    for pos in (1, 2, 3):
        hits = [i for i, f in enumerate(fins) if f == pos]
        out[pos] = hits[0] if len(hits) == 1 else None
    return out


class _Floor:
    def __init__(self) -> None:
        self.count = 0

    def nll(self, x: float) -> float:
        if x <= FLOOR:
            self.count += 1
            x = FLOOR
        return -math.log(x)


def race_scores(combo: Combo, fins: list, p_top2: np.ndarray | None = None) -> dict:
    """1 レースの券種別スコア。母集合外の券種は None。

    win/quinella/wide/trio は (NLL, 一様予測の NLL)。top2 と *_pairs は二値の集計値と (p, y) 配列。
    """
    n = len(fins)
    pod = podium(fins)
    ex = exclusion_reasons(fins)
    fl = _Floor()
    s: dict = {k: None for k in ("win", "quinella", "top2", "wide", "trio", "quinella_pairs", "wide_pairs")}
    i1, i2, i3 = pod[1], pod[2], pod[3]
    if ex["win"] is None:
        s["win"] = (fl.nll(combo.win[i1]), math.log(n))
    if ex["quinella"] is None:
        s["quinella"] = (fl.nll(combo.quinella[i1, i2]), math.log(math.comb(n, 2)))
        q = np.asarray(p_top2 if p_top2 is not None else combo.top2, dtype=float)
        y = np.array([1.0 if f is not None and f <= 2 else 0.0 for f in fins])
        fl.count += int(np.sum((q <= FLOOR) & (y == 1)) + np.sum((q >= 1 - FLOOR) & (y == 0)))
        qc = np.clip(q, FLOOR, 1 - FLOOR)
        u = min(max(2.0 / n, FLOOR), 1 - FLOOR)  # 一様予測: 各馬の連対確率 2/n
        s["top2"] = {
            "ll_sum": float(-np.sum(y * np.log(qc) + (1 - y) * np.log(1 - qc))),
            "uni_sum": float(-np.sum(y * math.log(u) + (1 - y) * math.log(1 - u))),
            "brier_sum": float(np.sum((q - y) ** 2)),
            "n": n,
            "p": q,
            "y": y,
        }
        pairs = list(combinations(range(n), 2))
        pp = np.array([combo.quinella[a, b] for a, b in pairs])
        yy = np.array([1.0 if {a, b} == {i1, i2} else 0.0 for a, b in pairs])
        s["quinella_pairs"] = {"brier_sum": float(np.sum((pp - yy) ** 2)), "n": len(pairs)}
    if ex["wide"] is None:
        hits = [tuple(sorted(t)) for t in combinations((i1, i2, i3), 2)]
        s["wide"] = (
            float(np.mean([fl.nll(combo.wide[a, b]) for a, b in hits])),
            math.log(math.comb(n, 2) / 3),
        )
        s["trio"] = (fl.nll(combo.trio[i1, i2, i3]), math.log(math.comb(n, 3)))
        pairs = list(combinations(range(n), 2))
        pp = np.array([combo.wide[a, b] for a, b in pairs])
        yy = np.array([1.0 if (a, b) in hits else 0.0 for a, b in pairs])
        s["wide_pairs"] = {"brier_sum": float(np.sum((pp - yy) ** 2)), "n": len(pairs), "p": pp, "y": yy}
    s["floored"] = fl.count
    return s


# 券種グループごとに一意でなければならない着順（単勝 / 馬連・連対 / ワイド・3連複）
_NEEDED_POSITIONS = {"win": (1,), "quinella": (1, 2), "wide": (1, 2, 3)}
# 予測に意味のある最小頭数。これ未満は的中確率が自明に 1（例: 3 頭立てのワイド・3連複）か、
# dump の行が欠けたレース（1 頭分しか行が無い等）なので評価しない
_MIN_FIELD = {"win": 2, "quinella": 3, "wide": 4}
BET_GROUP_LABELS = {"win": "単勝", "quinella": "馬連・連対", "wide": "ワイド・3連複"}
EXCLUSION_LABELS = {"field_too_small": "頭数不足", "dead_heat": "同着", "missing": "着順欠落"}


def exclusion_reasons(fins: list) -> dict:
    """券種グループごとの除外理由。母集合に入るなら None。

    優先順: "field_too_small"（行数が最小頭数未満）→ "dead_heat"（必要な着順のどれかが 2 頭以上）→
    "missing"（必要な着順のどれかが 0 頭 = 結果未取込・中止等）。
    """
    counts = Counter(f for f in fins if f is not None)
    out = {}
    for group, positions in _NEEDED_POSITIONS.items():
        if len(fins) < _MIN_FIELD[group]:
            out[group] = "field_too_small"
        elif any(counts[p] > 1 for p in positions):
            out[group] = "dead_heat"
        elif any(counts[p] == 0 for p in positions):
            out[group] = "missing"
        else:
            out[group] = None
    return out


# ---------- 集計・CI ----------

# 成分行列の列: 券種 NLL（和・一様の和・件数）→ 連対 → ペア Brier
_COLS = [
    "win_nll", "win_uni", "win_ok",
    "quinella_nll", "quinella_uni", "quinella_ok",
    "wide_nll", "wide_uni", "wide_ok",
    "trio_nll", "trio_uni", "trio_ok",
    "top2_ll", "top2_uni", "top2_brier", "top2_n",
    "qp_brier", "qp_n", "wp_brier", "wp_n",
]
_C = {c: i for i, c in enumerate(_COLS)}

METRICS = [
    "quinella_nll", "top2_nll", "wide_nll", "trio_nll", "win_nll",
    "quinella_r2", "top2_r2", "wide_r2", "trio_r2", "win_r2",
    "top2_brier", "quinella_brier", "wide_brier",
]
# 小さいほど良い指標（R² だけ大きいほど良い）
LOWER_IS_BETTER = {m: not m.endswith("_r2") for m in METRICS}


def judge_delta(metric: str, lo: float, hi: float) -> str:
    """系統 − baseline の差の CI から判定タグを返す。✅ = 改善側で 0 を跨がない、❌ = 悪化側で跨がない。"""
    if LOWER_IS_BETTER[metric]:
        better, worse = hi < 0, lo > 0
    else:
        better, worse = lo > 0, hi < 0
    return " ✅" if better else (" ❌" if worse else "")


def _components(scores: list[dict]) -> np.ndarray:
    m = np.zeros((len(scores), len(_COLS)))
    for r, s in enumerate(scores):
        for k in ("win", "quinella", "wide", "trio"):
            if s[k] is not None:
                m[r, _C[f"{k}_nll"]], m[r, _C[f"{k}_uni"]] = s[k]
                m[r, _C[f"{k}_ok"]] = 1.0
        if s["top2"] is not None:
            m[r, _C["top2_ll"]] = s["top2"]["ll_sum"]
            m[r, _C["top2_uni"]] = s["top2"]["uni_sum"]
            m[r, _C["top2_brier"]] = s["top2"]["brier_sum"]
            m[r, _C["top2_n"]] = s["top2"]["n"]
        if s["quinella_pairs"] is not None:
            m[r, _C["qp_brier"]] = s["quinella_pairs"]["brier_sum"]
            m[r, _C["qp_n"]] = s["quinella_pairs"]["n"]
        if s["wide_pairs"] is not None:
            m[r, _C["wp_brier"]] = s["wide_pairs"]["brier_sum"]
            m[r, _C["wp_n"]] = s["wide_pairs"]["n"]
    return m


def _metrics_from_sums(t: np.ndarray) -> dict:
    def ratio(a: float, b: float) -> float:
        return a / b if b > 0 else math.nan

    out = {}
    for k in ("win", "quinella", "wide", "trio"):
        out[f"{k}_nll"] = ratio(t[_C[f"{k}_nll"]], t[_C[f"{k}_ok"]])
        out[f"{k}_r2"] = 1.0 - ratio(t[_C[f"{k}_nll"]], t[_C[f"{k}_uni"]])
        out[f"{k}_n"] = int(round(t[_C[f"{k}_ok"]]))
    out["top2_nll"] = ratio(t[_C["top2_ll"]], t[_C["top2_n"]])
    out["top2_r2"] = 1.0 - ratio(t[_C["top2_ll"]], t[_C["top2_uni"]])
    out["top2_brier"] = ratio(t[_C["top2_brier"]], t[_C["top2_n"]])
    out["quinella_brier"] = ratio(t[_C["qp_brier"]], t[_C["qp_n"]])
    out["wide_brier"] = ratio(t[_C["wp_brier"]], t[_C["wp_n"]])
    return out


def aggregate(scores: list[dict]) -> dict:
    out = _metrics_from_sums(_components(scores).sum(axis=0))
    out["floored"] = sum(s["floored"] for s in scores)
    return out


def _boot_weights(n: int, n_boot: int, seed: int) -> np.ndarray:
    """レース単位の再標本を「各レースの採用回数」の行列 (n_boot, n) で表す。"""
    rng = np.random.default_rng(seed)
    return np.stack([np.bincount(rng.integers(0, n, size=n), minlength=n) for _ in range(n_boot)])


def boot_ci(
    scores_a: list[dict], scores_b: list[dict] | None, metric: str, n_boot: int = 1000, seed: int = 42
) -> tuple[float, float]:
    """metric(A)（B があれば metric(A) − metric(B)）の 95% CI。A/B は同一レースを同順で持つこと。"""
    ca = _components(scores_a)
    cb = _components(scores_b) if scores_b is not None else None
    if cb is not None and cb.shape != ca.shape:
        raise ValueError("対応ありブートストラップは同一レース集合（同長・同順）が前提です")
    w = _boot_weights(len(scores_a), n_boot, seed)
    ta, vals = w @ ca, np.empty(n_boot)
    tb = w @ cb if cb is not None else None
    for b in range(n_boot):
        v = _metrics_from_sums(ta[b])[metric]
        if tb is not None:
            v -= _metrics_from_sums(tb[b])[metric]
        vals[b] = v
    return float(np.nanquantile(vals, 0.025)), float(np.nanquantile(vals, 0.975))


# ---------- 入力 ----------


def _prob_cell(cell: str, what: str, path: str, lineno: int) -> float:
    try:
        v = float(cell)
    except ValueError as e:
        raise ValueError(f"{path}:{lineno} {what} が数値ではありません: {cell!r}") from e
    if not math.isfinite(v) or v < 0 or v > 1:
        raise ValueError(f"{path}:{lineno} {what} が確率の値域 [0,1] 外です: {cell!r}")
    return v


def load_probs(path: str) -> dict:
    """外部確率 TSV を {(race_id, horse_num): {"p_win", "p_top2"}} で読む。不正は即エラー。"""
    out: dict = {}
    with open(path, encoding="utf-8", newline="") as f:
        reader = csv.reader(f, delimiter="\t")
        header = next(reader, None) or []
        idx = {name.strip(): i for i, name in enumerate(header)}
        missing = [c for c in ("race_id", "horse_num", "p_win") if c not in idx]
        if missing:
            raise ValueError(f"{path}: 必須列がありません: {missing}")
        need = max(idx[c] for c in ("race_id", "horse_num", "p_win")) + 1
        for lineno, cells in enumerate(reader, start=2):
            if not cells or all(not c.strip() for c in cells):
                continue
            if len(cells) < need:
                raise ValueError(f"{path}:{lineno} 列が足りません（{len(cells)} 列・必要 {need} 列）")
            raw_num = cells[idx["horse_num"]].strip()
            if not _HORSE_NUM.match(raw_num):
                raise ValueError(f"{path}:{lineno} horse_num が 1 以上の整数ではありません: {raw_num!r}")
            horse_num = int(raw_num)
            key = (cells[idx["race_id"]].strip(), horse_num)
            if key in out:
                raise ValueError(f"{path}:{lineno} 重複行 {key}")
            top2 = None
            if "p_top2" in idx and idx["p_top2"] < len(cells) and cells[idx["p_top2"]].strip():
                top2 = _prob_cell(cells[idx["p_top2"]], "p_top2", path, lineno)
            out[key] = {"p_win": _prob_cell(cells[idx["p_win"]], "p_win", path, lineno), "p_top2": top2}
    # 連対を直接出す系統は全行で持つこと。一部だけだと、レースごとに連対の定義（直接 / Harville 由来）が
    # 黙って混ざり、主指標を系統の性能として解釈できなくなる。
    n_top2 = sum(1 for v in out.values() if v["p_top2"] is not None)
    if 0 < n_top2 < len(out):
        raise ValueError(f"{path}: p_top2 が一部の行（{n_top2}/{len(out)}）にしかありません。全行に入れるか列ごと空にしてください")
    return out


def validate_windows4(dev_from: str, dev_to: str, test_from: str, test_to: str) -> str | None:
    """dev/test 窓の検証。エラーなら理由文字列、正常なら None（dev は test より前・重複禁止）。"""
    names = ("--dev-from", "--dev-to", "--test-from", "--test-to")
    vals = (dev_from, dev_to, test_from, test_to)
    for name, v in zip(names, vals):
        # fromisoformat は 3.11+ で basic 形式（20250701）も通すが、窓は文字列比較なので拡張形式に限る
        try:
            ok = bool(_ISO_DATE.match(v)) and _date.fromisoformat(v) is not None
        except ValueError:
            ok = False
        if not ok:
            return f"{name} は YYYY-MM-DD 形式で指定してください: {v!r}"
    if dev_from > dev_to or test_from > test_to:
        return "窓の開始日が終了日より後です"
    if dev_to >= test_from:
        if dev_from > test_to:
            return f"dev 窓（{dev_from}〜）は test 窓（〜{test_to}）より前に置いてください"
        return f"dev 窓（〜{dev_to}）と test 窓（{test_from}〜）が重複しています"
    if dev_from <= FROZEN_EVAL_TO and dev_to >= FROZEN_EVAL_FROM:
        return (
            f"dev 窓が #703 で凍結した eval 窓（{FROZEN_EVAL_FROM}〜{FROZEN_EVAL_TO}）に重なっています。"
            "eval 窓は test としてだけ測り、ledger に記録してください"
        )
    return None


def in_window(d: str, lo: str, hi: str) -> bool:
    return lo <= d <= hi


# ---------- 系統と評価 ----------


@dataclass
class System:
    """確率系統。probs が None なら dump の model_win_pure（baseline）を使う。"""

    name: str
    probs: dict | None
    lam2: float = 1.0
    lam3: float = 1.0
    source: str = ""  # ledger に残す入力の来歴（外部 TSV のファイル名と sha256）

    @property
    def probs_index(self) -> dict:
        """race_id → その系統が確率を持つ (race_id, horse_num) の一覧（余剰行の検出用）。"""
        if self.probs is None:
            return {}
        if not hasattr(self, "_index"):
            idx: dict = {}
            for key in self.probs:
                idx.setdefault(key[0], []).append(key)
            self._index = idx
        return self._index


def system_probs(rows: list[dict], probs: dict | None) -> tuple[np.ndarray, np.ndarray | None] | str:
    """レース内全馬の (p_win, p_top2 or None)。欠ければ理由文字列を返す（母集合から外す）。

    "race_absent" = レースの行が 1 つも無い（系統のカバレッジ外）、"horse_partial" = 一部の馬だけ無い
    （確率の出し漏れの疑い）。
    """
    if probs is None:
        vals = [r.get("model_win_pure") for r in rows]
        if all(v is None for v in vals):
            return "race_absent"
        if any(v is None for v in vals):
            return "horse_partial"
        return np.array(vals, dtype=float), None
    got = [probs.get((r["race_id"], r["horse_num"])) for r in rows]
    if all(g is None for g in got):
        return "race_absent"
    if any(g is None for g in got):
        return "horse_partial"
    p = np.array([g["p_win"] for g in got])
    t2 = [g["p_top2"] for g in got]
    # load_probs が「全行あり / 全行なし」を保証しているので、レース内で混在はしない
    return p, (np.array(t2) if t2[0] is not None else None)


# 外部系統の p_win のレース内和がこれより外れたら警告に数える（正規化はする）
P_WIN_SUM_TOL = 0.01
# 直接出力の連対確率はレース内和がほぼ 2。頭数あたりこれ以上外れたら警告（定義違いの TSV の検出）
P_TOP2_SUM_TOL = 0.01


def evaluate(races: "OrderedDict[str, list[dict]]", systems: "OrderedDict[str, System]") -> dict:
    """全系統が確率を持つレースに揃えて、系統ごとのレース単位スコアを作る。

    excluded: 系統の欠落でレースごと外した件数（系統ごとに数える・1 レースで複数系統が欠ければ各々に計上）。
    bet_excluded: 評価対象レースのうち券種グループごとに母集合外とした件数（同着 / 着順欠落の理由別）。
    warnings: 正規化はしたが p_win の和が 1 から P_WIN_SUM_TOL 以上外れたレース数（系統別）。
    """
    excluded: Counter = Counter()
    bet_excluded: Counter = Counter()
    warnings: Counter = Counter()
    scores: dict = {name: [] for name in systems}
    race_ids = []
    for rid, rows in races.items():
        got = {name: system_probs(rows, sysm.probs) for name, sysm in systems.items()}
        missing = {name: sp for name, sp in got.items() if isinstance(sp, str)}
        for name, reason in missing.items():
            excluded[f"{reason}:{name}"] += 1
        # 外部 TSV にあって dump に無い馬（取消・除外で dump に行が無い等）。確率の和が正規化で消えるので数える
        nums = {r["horse_num"] for r in rows}
        for name, sysm in systems.items():
            if sysm.probs is not None:
                extra = sum(1 for (r_id, h) in sysm.probs_index.get(rid, ()) if h not in nums)
                if extra:
                    warnings[f"tsv_extra_horse:{name}"] += extra
        if missing:
            continue
        fins = [r.get("finishing_position") for r in rows]
        # 行数が馬番の最大値より少ない（取消・除外か、行の欠け。dump からは区別できない）
        if len(rows) < max(r["horse_num"] for r in rows):
            warnings["rows_lt_max_horse_num"] += 1
        for group, reason in exclusion_reasons(fins).items():
            if reason is not None:
                bet_excluded[(group, reason)] += 1
        for name, (p, t2) in got.items():
            sysm = systems[name]
            if sysm.probs is not None and abs(float(p.sum()) - 1.0) > P_WIN_SUM_TOL:
                warnings[f"p_win_sum_off:{name}"] += 1
            if t2 is not None and abs(float(t2.sum()) - 2.0) > P_TOP2_SUM_TOL * len(rows):
                warnings[f"p_top2_sum_off:{name}"] += 1
            scores[name].append(race_scores(combo_probs(p, sysm.lam2, sysm.lam3), fins, t2))
        race_ids.append(rid)
    return {
        "n_races": len(race_ids),
        "excluded": excluded,
        "bet_excluded": bet_excluded,
        "warnings": warnings,
        "scores": scores,
        "race_ids": race_ids,
    }


def evaluate_market(races: "OrderedDict[str, list[dict]]", lam2: float, lam3: float) -> list[dict]:
    """市場参考列（単勝オッズ → 割引 Harville）。オッズ不完全なレースは飛ばす（別 N）。"""
    out = []
    for rows in races.values():
        q = pe.market_probs(rows)
        if q is None:
            continue
        fins = [r.get("finishing_position") for r in rows]
        out.append(race_scores(combo_probs(np.array(q), lam2, lam3), fins))
    return out


def fidelity_pure_win_brier(races: "OrderedDict[str, list[dict]]") -> tuple[float, int]:
    """prob_eval の CORP 表（win × pure）と同じ母集合（勝者一意かつ市場オッズ完全）の Brier。"""
    ps, ys, n = [], [], 0
    for rows in races.values():
        if sum(1 for r in rows if r.get("finishing_position") == 1) != 1 or pe.market_probs(rows) is None:
            continue
        if any(r.get("model_win_pure") is None for r in rows):
            continue  # system_probs と同じく純モデル確率の欠けたレースは外す
        n += 1
        for r in rows:
            ps.append(float(r["model_win_pure"]))
            ys.append(pe._label(r, 1))
    if not ps:
        return math.nan, 0
    return float(np.mean((np.array(ps) - np.array(ys)) ** 2)), n


def corp_summary(scores: list[dict], key: str, n_boot: int, seed: int) -> dict | None:
    """連対（key=top2）・ワイドペア（key=wide_pairs）の CORP 分解と帯外点数。"""
    parts = [s[key] for s in scores if s[key] is not None]
    if not parts:
        return None
    p = np.concatenate([x["p"] for x in parts])
    y = np.concatenate([x["y"] for x in parts])
    d = pe.corp_decomposition(p, y)
    band = pe.reliability_band(p, y, n_boot=n_boot, seed=seed)
    d["outside"] = sum(1 for b in band if b["outside"])
    d["grid"] = len(band)
    return d


# ---------- レポート・ledger ----------


def append_ledger(path: str, label: str, lines: list[str]) -> None:
    """ledger（markdown）に 1 版分の節を追記する。既存の節は書き換えない（append-only）。"""
    with open(path, "a", encoding="utf-8") as f:
        f.write(f"\n## {label}\n\n")
        f.write("\n".join(lines).rstrip() + "\n")


def _fmt(v: float) -> str:
    return "—" if v is None or (isinstance(v, float) and math.isnan(v)) else f"{v:.5f}"


def report_window(
    title: str,
    races: "OrderedDict[str, list[dict]]",
    systems: "OrderedDict[str, System]",
    market_lam: tuple[float, float],
    n_boot: int,
    seed: int,
    fidelity_note: str = "参考値",
) -> tuple[list[str], dict]:
    """1 窓分のレポート行（markdown）と evaluate の結果を返す。

    fidelity_note: 忠実性サニティ行の突合先。prob_eval に同じ窓がある場合だけ突合を促す文言にする。
    """
    res = evaluate(races, systems)
    names = list(systems)
    base = names[0]
    lines = [f"### {title}", ""]
    ex = ", ".join(f"{k} {v}" for k, v in sorted(res["excluded"].items())) or "なし"
    lines.append(f"- 対象レース {res['n_races']}（窓内 {len(races)}・系統の欠落による除外: {ex}）")
    if res["n_races"] == 0:
        return lines + ["- 評価対象レースなし", ""], res
    agg = {n: aggregate(res["scores"][n]) for n in names}
    ns = agg[base]
    lines.append(
        "- 券種別 N: " + " / ".join(f"{k} {ns[f'{k}_n']}" for k in ("quinella", "wide", "trio", "win"))
        + "（連対は馬連と同じレース集合）"
    )
    reasons = []
    for group, label in BET_GROUP_LABELS.items():
        parts = "・".join(f"{EXCLUSION_LABELS[r]} {res['bet_excluded'][(group, r)]}" for r in EXCLUSION_LABELS)
        reasons.append(f"{label} {parts}")
    lines.append("- 券種別の除外（対象レースのうち母集合外）: " + " / ".join(reasons))
    if res["warnings"]:
        lines.append(
            "- 警告（評価は続行）: "
            + ", ".join(f"{k} {v}" for k, v in sorted(res["warnings"].items()))
            + f"（p_win_sum_off = レース内和が 1 から {P_WIN_SUM_TOL} 超外れ・p_top2_sum_off = 連対の和が 2 から"
            " 頭数×0.01 超外れ・tsv_extra_horse = TSV にあって dump に無い馬・rows_lt_max_horse_num = 行数が"
            "馬番の最大値より少ないレース〈取消・除外か行の欠け〉）"
        )
    floored = {n: agg[n]["floored"] for n in names if agg[n]["floored"]}
    if floored:
        lines.append(f"- 警告: 確率 ≤ {FLOOR} を floor（系統別件数 {floored}）")
    fb, fn = fidelity_pure_win_brier(races)
    lines.append(f"- 忠実性サニティ: pure win Brier（勝者一意かつ市場オッズ完全の {fn}R）= {fb:.6f}（{fidelity_note}）")
    header = ["指標", *names, *(f"Δ({n}−{base})" for n in names[1:])]
    lines += ["", "| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    for m in METRICS:
        cells = []
        for n in names:
            lo, hi = boot_ci(res["scores"][n], None, m, n_boot=n_boot, seed=seed)
            cells.append(f"{_fmt(agg[n][m])} [{_fmt(lo)}, {_fmt(hi)}]")
        for n in names[1:]:
            lo, hi = boot_ci(res["scores"][n], res["scores"][base], m, n_boot=n_boot, seed=seed)
            cells.append(f"{agg[n][m] - agg[base][m]:+.5f} [{lo:+.5f}, {hi:+.5f}]{judge_delta(m, lo, hi)}")
        lines.append(f"| {m} | " + " | ".join(cells) + " |")
    lines += ["", "CORP 分解（Brier = MCB − DSC + UNC・点推定。帯外点は 90% pointwise の consistency band）:", ""]
    lines += ["| 対象 | 系統 | n | Brier | MCB | DSC | UNC | 帯外点 |", "|---|---|---|---|---|---|---|---|"]
    for key, label in (("top2", "連対"), ("wide_pairs", "ワイドペア")):
        for n in names:
            d = corp_summary(res["scores"][n], key, n_boot=min(n_boot, 100), seed=seed)
            if d is None:
                continue
            lines.append(
                f"| {label} | {n} | {d['n']} | {d['brier']:.5f} | {d['mcb']:.5f} | {d['dsc']:.5f}"
                f" | {d['unc']:.5f} | {d['outside']}/{d['grid']} |"
            )
    mk = evaluate_market(races, *market_lam)
    if mk:
        ma = aggregate(mk)
        lines += [
            "",
            f"市場参考列（単勝オッズ → Harville λ={market_lam[0]},{market_lam[1]}・オッズ完全な {len(mk)}R の別母集合・"
            "点推定のみ・ゲートにしない。`win_odds` はスナップショットと確定オッズが混在）: "
            + " / ".join(
                f"{m} {_fmt(ma[m])}（N {ma[m.split('_')[0] + '_n'] if not m.startswith('top2') else ma['quinella_n']}）"
                for m in ("quinella_nll", "top2_nll", "wide_nll", "trio_nll", "win_nll")
            ),
        ]
    return lines + [""], res


# 系統名は markdown の表見出しに入るので、表や見出しを壊さない文字に限る
_SYSTEM_NAME = re.compile(r"^[A-Za-z0-9_.-]+$")


def _parse_kv(items: list[str], what: str) -> dict:
    """NAME=VALUE の繰り返し引数を dict にする。同じ NAME の重複は黙って後勝ちにせずエラーにする。"""
    out = {}
    for it in items:
        if "=" not in it:
            raise ValueError(f"{what} は NAME=VALUE で指定してください: {it!r}")
        k, v = it.split("=", 1)
        k = k.strip()
        if k in out:
            raise ValueError(f"{what} の {k!r} が重複しています")
        out[k] = v.strip()
    return out


def parse_lambda(value: str) -> tuple[float, float]:
    """`L2,L3` を検証して返す。λ は有限かつ正（0 は 0**0=1、負は 0 に無限大を与えるため不可）。"""
    parts = value.split(",")
    if len(parts) != 2:
        raise ValueError(f"L2,L3 の形式で指定してください: {value!r}")
    lam = tuple(float(x) for x in parts)
    if not all(math.isfinite(v) and v > 0 for v in lam):
        raise ValueError(f"λ は有限の正の数にしてください: {value!r}")
    return lam  # type: ignore[return-value]


def _git_run(*args: str) -> str:
    # cwd でなくスクリプトの置き場所で引く（別 worktree から実行しても計測したコードの版を記録する）
    return subprocess.run(
        ["git", *args], capture_output=True, text=True, check=True,
        cwd=os.path.dirname(os.path.abspath(__file__)),
    ).stdout.strip()


def code_version() -> tuple[str, bool]:
    """(短縮 sha, 評価コードに未コミットの変更があるか)。

    見るのは評価結果を左右する scripts/predict-check/ だけ（並走セッションの無関係な変更で記録を止めない）。
    追跡外の .py も import されうるので変更とみなす。git が引けなければ版を保証できないので「変更あり」と
    扱う（記録はフェイルクローズ）。
    """
    try:
        sha = _git_run("rev-parse", "--short", "HEAD")
        top = _git_run("rev-parse", "--show-toplevel")
        status = _git_run("-C", top, "status", "--porcelain", "--untracked-files=all", "--", "scripts/predict-check")
    except (OSError, subprocess.CalledProcessError):
        return "unknown", True
    dirty = any(
        not line.startswith("??") or line.rstrip().endswith(".py") for line in status.splitlines() if line.strip()
    )
    return sha, dirty


def _file_sha(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:12]


# ledger に書くファイル名（basename）。改行・バッククォート・| は節や表を壊すので拒否する
_UNSAFE_NAME = re.compile(r"[\r\n`|]")


def validate_basename(path: str) -> str | None:
    name = os.path.basename(path)
    if not name or _UNSAFE_NAME.search(name):
        return f"ファイル名に改行・バッククォート・| を含めないでください: {name!r}"
    return None


def validate_label(label: str) -> str | None:
    """ledger の節見出し。1 行で、見出し記号から始めない（ledger に節を注入しない）。"""
    if "\n" in label or "\r" in label or not label.strip():
        return "--label は空でない 1 行にしてください"
    if label.lstrip().startswith("#"):
        return "--label を # で始めないでください"
    return None


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("dump", help="analyze backtest --dump-features の TSV（56 列版）")
    ap.add_argument("--system", action="append", default=[], help="NAME=PATH（外部確率 TSV・複数可）")
    ap.add_argument("--lambda", dest="lambdas", action="append", default=[],
                    help="NAME=L2,L3（系統の割引 λ。pure / market / --system の名前に指定可。既定 1,1）")
    ap.add_argument("--dev-from", default="2025-07-01")
    ap.add_argument("--dev-to", default="2025-12-31")
    ap.add_argument("--test-from", default="2026-01-01")
    ap.add_argument("--test-to", default="2026-08-31")
    ap.add_argument("--windows", choices=["dev", "test", "both"], default="dev",
                    help="test 窓は版の節目にだけ測り、必ず --ledger に記録する（既定 dev のみ）")
    ap.add_argument("--bootstrap", type=int, default=1000)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--ledger", help="追記先の ledger（markdown）")
    ap.add_argument("--label", help="ledger の節見出し（--ledger 指定時は必須・1 行）")
    args = ap.parse_args(argv)

    err = validate_windows4(args.dev_from, args.dev_to, args.test_from, args.test_to)
    if err:
        ap.error(err)
    if args.windows != "dev" and not args.ledger:
        # test 窓を見たことが必ず記録に残るようにする（記録の無い覗き見 → 調整の反復を防ぐ）
        ap.error("--windows test / both は --ledger と --label を付けて実行してください（test 窓の計測は必ず記録する）")
    if args.ledger:
        if not args.label:
            ap.error("--ledger には --label が必要です")
        err = validate_label(args.label)
        if err:
            ap.error(err)
    sha, dirty = code_version()
    if args.ledger and dirty:
        ap.error(
            "scripts/predict-check/ に未コミットの変更があるか、git の版を確認できません。"
            "ledger に載る git 版と計測コードがずれるので、コミットしてから記録してください"
        )

    try:
        lam_args = _parse_kv(args.lambdas, "--lambda")
        sys_paths = _parse_kv(args.system, "--system")
    except ValueError as e:
        ap.error(str(e))
    try:
        lams = {k: parse_lambda(v) for k, v in lam_args.items()}
    except ValueError as e:
        ap.error(f"--lambda: {e}")
    for path in [args.dump, *sys_paths.values()]:
        err = validate_basename(path)
        if err:
            ap.error(err)
    for name in sys_paths:
        if name in ("pure", "market"):
            ap.error(f"系統名 {name!r} は予約済みです")
        if not _SYSTEM_NAME.match(name):
            ap.error(f"系統名 {name!r} は英数字と _ . - だけにしてください")
    unknown = sorted(set(lams) - {"pure", "market", *sys_paths})
    if unknown:
        ap.error(f"--lambda に存在しない系統名があります: {unknown}")
    systems: "OrderedDict[str, System]" = OrderedDict()
    systems["pure"] = System("pure", None, *lams.get("pure", (1.0, 1.0)))
    for name, path in sys_paths.items():
        systems[name] = System(
            name, load_probs(path), *lams.get(name, (1.0, 1.0)),
            source=f"`{os.path.basename(path)}` sha256 先頭 12 桁 {_file_sha(path)}",
        )

    rows = pe.load_dump(args.dump)
    table = pe.build_race_table(rows)
    windows = []
    if args.windows in ("dev", "both"):
        windows.append(("dev", args.dev_from, args.dev_to))
    if args.windows in ("test", "both"):
        windows.append(("test", args.test_from, args.test_to))

    lines = [
        f"- 計測日: {_date.today().isoformat()} / git {sha}{'-dirty' if dirty else ''}",
        f"- dump: `{os.path.basename(args.dump)}`（sha256 先頭 12 桁 {_file_sha(args.dump)}）",
        "- 系統: " + ", ".join(
            f"{s.name}（λ={s.lam2},{s.lam3}{'・' + s.source if s.source else ''}）" for s in systems.values()
        ),
        f"- 市場参考列 λ={lams.get('market', (1.0, 1.0))[0]},{lams.get('market', (1.0, 1.0))[1]}"
        f" / bootstrap {args.bootstrap} / seed {args.seed}",
        "",
    ]
    frozen_test = (args.test_from, args.test_to) == (FROZEN_EVAL_FROM, FROZEN_EVAL_TO)
    for wname, lo, hi in windows:
        races = OrderedDict(
            (rid, rr) for rid, rr in table.races.items() if in_window(rr[0]["date"], lo, hi)
        )
        # prob_eval の eval 窓（--eval-from 以降・上限なし）と同じ母集合になるのは、test が凍結 eval 窓で
        # dump が test-to までの場合だけ。それ以外は照合先が無いので参考値と書く
        note = (
            "prob_eval の eval 窓の win×pure と一致すること（dump の終端が test-to のとき）"
            if wname == "test" and frozen_test
            else "参考値。prob_eval に同じ窓は無い"
        )
        wlines, res = report_window(
            f"{wname}（{lo}〜{hi}）", races, systems, lams.get("market", (1.0, 1.0)), args.bootstrap, args.seed, note
        )
        if args.ledger and res["n_races"] == 0:
            ap.error(f"{wname} 窓の評価対象レースが 0 件です（系統の被覆や窓指定を確認してください）。ledger には記録しません")
        lines += wlines
    print("\n".join(lines))
    if args.ledger:
        append_ledger(args.ledger, args.label, lines)
        print(f"\nledger に追記しました: {args.ledger}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
