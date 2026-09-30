#!/usr/bin/env python3
"""独立確率 × 連系市場の Benter 型合成で、独立確率にエッジ（α>0）があるかを判定する（#722）。

モデル（券種ごと・1 レースの全組合せ c で正規化する条件付きロジット）:
  p_c ∝ exp(α·ln f_c + β·ln π_c)
  f = 独立確率（#720 の割引 PL top-k。確率 TSV の p_win とレースごとの λ から `prob_ledger.combo_probs` で作る）
  π = 市場確率（確定オッズ。π_c = (1/O_c) / Σ(1/O)。レース内で正規化するので控除は消える。
      ワイドはオッズ帯の中点。W=3 の正規化は、確率として 1 レースの和 1 にそろえる形と同じ）
α>0 が有意で、かつ市場単独（α=0・β=1＝π そのもの）より当てはまりが良いときだけ、独立確率は市場の持たない
情報を持つ。ズレに直接賭けた ROI は判定に使わない（Benter 1994 表 3/4）。

尤度:
  馬連・3連複: 的中の組合せ 1 つのカテゴリカル尤度。
  ワイド: 的中 3 組それぞれの対数尤度の和（複合尤度）。3 組は同じ決着から出るので独立ではない。
          Hessian の SE は分散を過小に見積もるため、ワイドの CI はレース単位のブートストラップだけで出す。
3連複の無投票の組合せ（確定後も票が無い）: π の下限として、そのレースで売れた組合せの最小 π の半分を置き、
  レース内で正規化し直す（QA Q2・dev で「除外して正規化」を感度分析として並べる）。

窓（backtest.md 評価プロトコル・#723）: fit = dev 窓（既定 2025-07-01〜2025-12-31）で α・β を推定し、
定義の調整もここだけで行う。eval = test 窓（2026-01-01〜2026-08-31）は **1 回だけ**測り、`--ledger` に記録する。

判定（事前に固定）: 「エッジあり」は次をすべて満たすとき。
  1. fit 窓の α̂ の 95% CI（レース単位ブートストラップ）が 0 を跨がず正
  2. eval 窓で α を測り直しても 95% CI が 0 を跨がず正（確認のための再推定。調整には使わない）
  3. eval 窓で、fit 窓の (α̂, β̂) の合成が市場単独より良い: ΔR² の CI が 0 より上、かつ NLL 差の CI が 0 より下
ROI は参考値（p·O > 1 の組を 1 単位ずつ買った実現回収率。ワイドの払戻はオッズ帯の下限で近似）だけを出す。

入力:
  --probs    確率 TSV（race_id・horse_num・p_win・lam2・lam3。`scripts/harness/train_pl_topk.py` の出力）
  --odds     確定オッズ TSV（#721 の final_odds.tsv。race_id・bet_type・combination_key・odds・odds_high）
  --results  着順（`scripts/harness/pl_features_extract.sql` の出力 TSV。race_id・date・horse_num・status・finishing_position）

使い方:
  python3 scripts/predict-check/benter_alpha.py --probs k3.tsv --odds final_odds.tsv --results pl_raw.tsv
  python3 scripts/predict-check/benter_alpha.py ... --windows test \\
      --ledger docs-original/722-benter-alpha-exotics.md --label "test: ..."
"""

from __future__ import annotations

import argparse
import csv
import itertools
import math
import os
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field

import numpy as np

import prob_ledger as pl

BET_TYPES = ("quinella", "wide", "trio")
K = {"quinella": 2, "wide": 2, "trio": 3}
STARTERS = {"finished", "did_not_finish"}
FLOOR = pl.FLOOR
TAKEOUT = {"quinella": 0.225, "wide": 0.225, "trio": 0.25}  # 参考（ROI は実オッズで測るので控除は既に入っている）


# ---------- 市場確率 ----------


def odds_value(row: dict, bet_type: str, wide_low: bool = False) -> float:
    """確定オッズ 1 行の代表値。ワイドは帯の中点（wide_low なら下限）。"""
    low = float(row["odds"])
    if bet_type != "wide" or wide_low:
        return low
    return (low + float(row["odds_high"])) / 2


def market_probs(combos: list, odds: dict, floor: bool) -> tuple[np.ndarray, int]:
    """組合せの市場確率（レース内で和 1）と、下限を置いた組の数。

    売れていない組合せ（odds に無い）は、floor なら売れた組の最小 π の半分を置き、floor でなければ 0 にする。
    """
    inv = np.array([1.0 / odds[c] if c in odds else 0.0 for c in combos])
    sold = inv > 0
    if not sold.any():
        raise ValueError("売れた組合せがありません")
    pi = inv / inv[sold].sum()
    n_floor = int((~sold).sum())
    if floor and n_floor:
        pi[~sold] = pi[sold].min() / 2
        pi = pi / pi.sum()
    return pi, (n_floor if floor else 0)


# ---------- レースの組み立て ----------


@dataclass
class Race:
    combos: list
    log_f: np.ndarray
    log_pi: np.ndarray
    winners: list
    odds: np.ndarray | None = None  # 組ごとの払戻の近似（ROI 用・売れていない組は nan）
    reason: str | None = None
    winner_unsold: bool = False
    n_floor: int = 0


def _excluded(reason: str) -> Race:
    return Race(combos=[], log_f=np.zeros(0), log_pi=np.zeros(0), winners=[], reason=reason)


def build_race(bet_type: str, horses: list, probs: dict, lam: tuple, fins: dict, odds: dict,
               floor: bool, payout: dict | None = None) -> Race:
    """1 レース・1 券種の組合せと f・π・的中を作る。母集合外なら reason 付きで返す。

    horses: 出走馬（完走・中止）の馬番。probs: 馬番 → p_win。fins: 馬番 → 着順（無ければ None）。
    odds: 組合せ（昇順の馬番タプル）→ 確率に使うオッズ。payout: 組合せ → ROI 用の払戻の近似（省略時は odds）。
    """
    k = K[bet_type]
    if not odds:
        return _excluded("no_odds")
    if any(h not in probs for h in horses):
        return _excluded("probs_missing")
    hs = sorted(horses)
    if not {h for c in odds for h in c} <= set(hs):
        return _excluded("market_horse_not_in_probs")
    top = []
    for pos in range(1, (3 if bet_type != "quinella" else 2) + 1):
        hit = [h for h in hs if fins.get(h) == pos]
        if len(hit) != 1:
            return _excluded("podium_not_unique")
        top.append(hit[0])
    combos = list(itertools.combinations(hs, k))
    idx = {h: i for i, h in enumerate(hs)}
    cp = pl.combo_probs(np.array([probs[h] for h in hs]), lam[0], lam[1])
    mat = {"quinella": cp.quinella, "wide": cp.wide, "trio": cp.trio}[bet_type]
    f = np.array([mat[tuple(idx[h] for h in c)] for c in combos])
    f = f / f.sum()
    if bet_type == "quinella":
        win_combos = [tuple(sorted(top))]
    elif bet_type == "wide":
        win_combos = [tuple(sorted(p)) for p in itertools.combinations(top, 2)]
    else:
        win_combos = [tuple(sorted(top))]
    unsold = any(c not in odds for c in win_combos)
    pi, n_floor = market_probs(combos, odds, floor)
    if not floor:
        keep = [i for i, c in enumerate(combos) if c in odds]
        if unsold:
            return _excluded("winner_unsold")
        combos = [combos[i] for i in keep]
        f = f[keep] / f[keep].sum()
        pi = pi[keep]
    pay = payout if payout is not None else odds
    return Race(
        combos=combos,
        log_f=np.log(np.maximum(f, FLOOR)),
        log_pi=np.log(np.maximum(pi, FLOOR)),
        winners=[combos.index(c) for c in win_combos],
        odds=np.array([pay.get(c, np.nan) for c in combos]),
        winner_unsold=unsold,
        n_floor=n_floor,
    )


# ---------- 推定（条件付きロジット・全レースをまとめてベクトル化） ----------


@dataclass
class Packed:
    x: np.ndarray  # (2, N)  ln f, ln π
    starts: np.ndarray  # (R,) 各レースの先頭位置
    seg: np.ndarray  # (N,) 行 → レース
    win_rows: np.ndarray  # 的中した行（ワイドは 1 レース 3 行）
    n_win: np.ndarray  # (R,) レースごとの的中数
    n_combos: np.ndarray  # (R,)
    races: list = field(default_factory=list)


def pack(races: list) -> Packed:
    sizes = np.array([len(r.combos) for r in races])
    starts = np.concatenate([[0], np.cumsum(sizes)[:-1]]).astype(int)
    x = np.stack([np.concatenate([r.log_f for r in races]), np.concatenate([r.log_pi for r in races])])
    seg = np.repeat(np.arange(len(races)), sizes)
    win_rows = np.concatenate([starts[i] + np.array(r.winners, dtype=int) for i, r in enumerate(races)])
    n_win = np.array([len(r.winners) for r in races])
    return Packed(x=x, starts=starts, seg=seg, win_rows=win_rows, n_win=n_win, n_combos=sizes, races=races)


def _softmax(pk: Packed, theta: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    z = theta @ pk.x
    zmax = np.maximum.reduceat(z, pk.starts)
    e = np.exp(z - zmax[pk.seg])
    s = np.add.reduceat(e, pk.starts)
    return e / s[pk.seg], z - zmax[pk.seg] - np.log(s)[pk.seg]


def race_nll(pk: Packed, theta: np.ndarray) -> np.ndarray:
    """レースごとの負の対数尤度（ワイドは的中 3 組の和）。"""
    _, logp = _softmax(pk, theta)
    seg_w = pk.seg[pk.win_rows]
    return -np.bincount(seg_w, weights=logp[pk.win_rows], minlength=len(pk.starts))


def uniform_nll(pk: Packed) -> np.ndarray:
    """一様予測（全組合せ等確率）の NLL。擬似 R² の基準。"""
    return pk.n_win * np.log(pk.n_combos)


def pseudo_r2(nll: np.ndarray, unll: np.ndarray, w: np.ndarray | None = None) -> float:
    w = np.ones(len(nll)) if w is None else w
    return float(1.0 - (w * nll).sum() / (w * unll).sum())


def fit(pk: Packed, w: np.ndarray | None = None, max_iter: int = 60) -> tuple[np.ndarray, np.ndarray, float]:
    """(α, β) を Newton 法で最尤推定（レース重み w は bootstrap 用）。返り値: (θ̂, Hessian SE, logL)。"""
    w = np.ones(len(pk.starts)) if w is None else w
    theta = np.array([0.0, 1.0])  # 市場単独から始める（logL は凹なので初期値に依らない）
    hess = np.zeros((2, 2))
    for _ in range(max_iter):
        p, _ = _softmax(pk, theta)
        mean = np.stack([np.add.reduceat(pk.x[j] * p, pk.starts) for j in range(2)])  # (2, R)
        xs = pk.x[:, pk.win_rows]
        seg_w = pk.seg[pk.win_rows]
        grad = np.array([(w * (np.bincount(seg_w, weights=xs[j], minlength=len(w)) - pk.n_win * mean[j])).sum()
                         for j in range(2)])
        cen = pk.x - mean[:, pk.seg]
        wr = (w * pk.n_win)[pk.seg] * p
        hess = -np.array([[(wr * cen[a] * cen[b]).sum() for b in range(2)] for a in range(2)])
        step = np.linalg.solve(hess - 1e-9 * np.eye(2), grad)
        theta = theta - step
        if np.abs(step).max() < 1e-10:
            break
    else:
        raise RuntimeError(f"Newton 法が {max_iter} 反復で収束しません")
    se = np.sqrt(np.diag(np.linalg.inv(-hess)))
    return theta, se, float(-(w * race_nll(pk, theta)).sum())


def _boot_counts(n: int, n_boot: int, seed: int) -> np.ndarray:
    rng = np.random.default_rng(seed)
    return rng.multinomial(n, np.full(n, 1.0 / n), size=n_boot).astype(float)


def boot_theta(pk: Packed, n_boot: int, seed: int) -> np.ndarray:
    """レース単位ブートストラップで (α, β) を推定し直した分布（n_boot × 2）。"""
    return np.array([fit(pk, w)[0] for w in _boot_counts(len(pk.starts), n_boot, seed)])


def paired_ci(x: np.ndarray, y: np.ndarray, d: np.ndarray, n_boot: int, seed: int) -> tuple[float, float]:
    """Σ(x − y) / Σd の 95% CI（レース単位・同じ再標本で x と y を引く対応ありブートストラップ）。"""
    ws = _boot_counts(len(x), n_boot, seed)
    stats = (ws @ (x - y)) / (ws @ d)
    return float(np.percentile(stats, 2.5)), float(np.percentile(stats, 97.5))


def roi_reference(pk: Packed, theta: np.ndarray, n_boot: int, seed: int) -> tuple[float, int, float, float]:
    """p·O > 1 の組を 1 単位ずつ買った実現回収率（参考値）と点数、95% CI。"""
    p, _ = _softmax(pk, theta)
    odds = np.concatenate([r.odds if r.odds is not None else np.full(len(r.combos), np.nan) for r in pk.races])
    buy = np.nan_to_num(p * odds, nan=0.0) > 1.0
    hit = np.zeros(len(p), dtype=bool)
    hit[pk.win_rows] = True
    stake = np.bincount(pk.seg, weights=buy.astype(float), minlength=len(pk.starts))
    ret = np.bincount(pk.seg, weights=np.where(buy & hit, odds, 0.0), minlength=len(pk.starts))
    if stake.sum() == 0:
        return float("nan"), 0, float("nan"), float("nan")
    lo, hi = paired_ci(ret, np.zeros_like(ret), stake, n_boot, seed)
    return float(ret.sum() / stake.sum()), int(stake.sum()), lo, hi


# ---------- 入力 ----------


def load_results(path: str) -> tuple[dict, dict, dict]:
    """着順 TSV → (race_id → 日付, race_id → 出走馬番, race_id → {馬番: 着順})。"""
    date: dict = {}
    horses: dict = defaultdict(list)
    fins: dict = defaultdict(dict)
    with open(path, encoding="utf-8", newline="") as f:
        for lineno, r in enumerate(csv.DictReader(f, delimiter="\t"), start=2):
            rid = r["race_id"]
            if date.setdefault(rid, r["date"]) != r["date"]:
                raise ValueError(f"{path}:{lineno} 同じレースで日付が違います: {rid}")
            if r["status"] in STARTERS:
                h = int(r["horse_num"])
                horses[rid].append(h)
                fins[rid][h] = int(r["finishing_position"]) if r["finishing_position"] else None
    return date, horses, fins


def load_odds(path: str, wide_low: bool) -> tuple[dict, dict]:
    """確定オッズ TSV → ((race_id, bet_type) → {組合せ: 確率用オッズ}, 同 → {組合せ: ROI 用の払戻近似})。"""
    prob_odds: dict = defaultdict(dict)
    payout: dict = defaultdict(dict)
    with open(path, encoding="utf-8", newline="") as f:
        for r in csv.DictReader(f, delimiter="\t"):
            bt = r["bet_type"]
            if bt not in K:
                continue
            c = tuple(int(x) for x in r["combination_key"].split("-"))
            if list(c) != sorted(c) or len(c) != K[bt]:
                raise ValueError(f"組合せキーが不正です: {r['race_id']} {bt} {r['combination_key']}")
            prob_odds[(r["race_id"], bt)][c] = odds_value(r, bt, wide_low)
            payout[(r["race_id"], bt)][c] = float(r["odds"])  # ワイドは下限（保守的な近似）
    return prob_odds, payout


def collect(bet_type: str, lo: str, hi: str, date: dict, horses: dict, fins: dict, probs: dict,
            prob_odds: dict, payout: dict, floor: bool) -> tuple[list, Counter]:
    by_race: dict = defaultdict(dict)
    lam: dict = {}
    for (rid, h), v in probs.items():
        by_race[rid][h] = v["p_win"]
        lam[rid] = v["lam"] or (1.0, 1.0)
    races, why = [], Counter()
    for rid in sorted(r for r in by_race if pl.in_window(date.get(r, ""), lo, hi)):
        r = build_race(bet_type, horses[rid], by_race[rid], lam[rid], fins[rid],
                       prob_odds.get((rid, bet_type), {}), floor, payout.get((rid, bet_type), {}))
        if r.reason:
            why[r.reason] += 1
        else:
            races.append(r)
    return races, why


# ---------- 報告 ----------


def validate_args(dev_from: str, dev_to: str, test_from: str, test_to: str, windows: str,
                  ledger: str | None, label: str | None) -> str | None:
    err = pl.validate_windows4(dev_from, dev_to, test_from, test_to)
    if err:
        return err
    if windows == "test" and (not ledger or not label):
        return "test 窓は 1 回だけ測って記録する: --ledger と --label を指定してください"
    return None


def judge(fit_alpha_lo: float, eval_alpha_lo: float, dr2_lo: float, dnll_hi: float) -> bool:
    """事前に固定した判定: fit・eval 両窓の α の CI が 0 より上、かつ eval 窓で合成が市場単独より良い。"""
    return bool(fit_alpha_lo > 0 and eval_alpha_lo > 0 and dr2_lo > 0 and dnll_hi < 0)


def _ci(lo: float, hi: float) -> str:
    return f"[{lo:+.4f}, {hi:+.4f}]"


def report_bet(bet_type: str, fit_races: list, fit_why: Counter, eval_races: list | None, eval_why: Counter | None,
               n_boot: int, seed: int) -> tuple[list[str], dict]:
    out = [f"### {bet_type}", ""]
    pk = pack(fit_races)
    theta, se, _ = fit(pk)
    bt = boot_theta(pk, n_boot, seed)
    a_lo, a_hi = np.percentile(bt[:, 0], [2.5, 97.5])
    b_lo, b_hi = np.percentile(bt[:, 1], [2.5, 97.5])
    un = uniform_nll(pk)
    out.append(f"- fit 窓: {len(fit_races)}R（除外 {dict(fit_why) or 'なし'}）・下限を置いた組 "
               f"{sum(r.n_floor for r in fit_races)}・的中が無投票の組 {sum(r.winner_unsold for r in fit_races)}R")
    se_txt = "（複合尤度のため Hessian SE は出さない）" if bet_type == "wide" else f"・Hessian SE α {se[0]:.4f} / β {se[1]:.4f}"
    out.append(f"- α̂ = {theta[0]:+.4f} {_ci(a_lo, a_hi)}・β̂ = {theta[1]:+.4f} {_ci(b_lo, b_hi)}（bootstrap {n_boot}）{se_txt}")
    out.append(f"- fit 窓の擬似 R²（参考・同じ窓で推定）: 市場 {pseudo_r2(race_nll(pk, np.array([0.0, 1.0])), un):.4f} / "
               f"独立 {pseudo_r2(race_nll(pk, np.array([1.0, 0.0])), un):.4f} / 合成 {pseudo_r2(race_nll(pk, theta), un):.4f}")
    res = {"alpha": theta[0], "alpha_ci": (a_lo, a_hi)}
    if eval_races is not None:
        ek = pack(eval_races)
        eu = uniform_nll(ek)
        nm, nf, nb = (race_nll(ek, np.array(t)) for t in ([0.0, 1.0], [1.0, 0.0], theta))
        d_lo, d_hi = paired_ci(nm, nb, eu, n_boot, seed)  # ΔR² = Σ(NLL_市場 − NLL_合成) / Σ NLL_一様
        l_lo, l_hi = paired_ci(nb, nm, ek.n_win.astype(float), n_boot, seed)  # 的中 1 組あたりの NLL 差
        et, _, _ = fit(ek)
        ebt = boot_theta(ek, n_boot, seed)
        ea_lo, ea_hi = np.percentile(ebt[:, 0], [2.5, 97.5])
        roi, n_bet, r_lo, r_hi = roi_reference(ek, theta, n_boot, seed)
        dr2 = pseudo_r2(nb, eu) - pseudo_r2(nm, eu)
        dnll = float((nb - nm).sum() / ek.n_win.sum())
        out += [
            f"- eval 窓: {len(eval_races)}R（除外 {dict(eval_why) or 'なし'}）・下限を置いた組 "
            f"{sum(r.n_floor for r in eval_races)}・的中が無投票の組 {sum(r.winner_unsold for r in eval_races)}R",
            f"- eval 窓の擬似 R²: 市場 {pseudo_r2(nm, eu):.4f} / 独立 {pseudo_r2(nf, eu):.4f} / 合成（fit の θ̂） {pseudo_r2(nb, eu):.4f}",
            f"- ΔR²（合成 − 市場） = {dr2:+.5f} {_ci(d_lo, d_hi)}・NLL 差（合成 − 市場・的中 1 組あたり） = {dnll:+.5f} {_ci(l_lo, l_hi)}",
            f"- eval 窓で測り直した α = {et[0]:+.4f} {_ci(ea_lo, ea_hi)}・β = {et[1]:+.4f}（確認用。調整には使わない）",
            f"- 参考 ROI（p·O > 1 を 1 単位ずつ・{n_bet} 点）: {roi:.3f} {_ci(r_lo, r_hi)}（控除率 {TAKEOUT[bet_type]:.1%}）",
        ]
        edge = judge(a_lo, ea_lo, d_lo, l_hi)
        out.append(f"- 判定: **{'エッジあり' if edge else 'エッジなし'}**（fit α の CI > 0: {a_lo > 0} / eval α の CI > 0: {ea_lo > 0} / "
                   f"ΔR² の CI > 0: {d_lo > 0} / NLL 差の CI < 0: {l_hi < 0}）")
        res.update(edge=edge)
    out.append("")
    return out, res


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--probs", required=True)
    ap.add_argument("--odds", required=True)
    ap.add_argument("--results", required=True)
    ap.add_argument("--bet-types", default=",".join(BET_TYPES))
    ap.add_argument("--dev-from", default="2025-07-01")
    ap.add_argument("--dev-to", default="2025-12-31")
    ap.add_argument("--test-from", default=pl.FROZEN_EVAL_FROM)
    ap.add_argument("--test-to", default=pl.FROZEN_EVAL_TO)
    ap.add_argument("--windows", choices=("dev", "test"), default="dev",
                    help="dev = fit 窓だけで推定（反復用）/ test = fit で推定して eval 窓を 1 回測る（--ledger 必須）")
    ap.add_argument("--trio-no-floor", action="store_true", help="感度分析（dev のみ）: 3連複の無投票の組を除外して正規化")
    ap.add_argument("--wide-low", action="store_true", help="感度分析（dev のみ）: ワイドの π をオッズ帯の下限で作る")
    ap.add_argument("--bootstrap", type=int, default=1000)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--ledger")
    ap.add_argument("--label")
    args = ap.parse_args(argv)
    err = validate_args(args.dev_from, args.dev_to, args.test_from, args.test_to, args.windows, args.ledger, args.label)
    if not err and args.windows == "test" and (args.trio_no_floor or args.wide_low):
        err = "感度分析の定義変更は dev 窓だけで行う（test 窓は事前に固定した定義で 1 回だけ測る）"
    bets = [b.strip() for b in args.bet_types.split(",") if b.strip()]
    if not err and any(b not in K for b in bets):
        err = f"--bet-types は {BET_TYPES} から選んでください"
    if not err and args.label and (e := pl.validate_label(args.label)):
        err = e
    version, dirty = pl.code_version()
    if not err and args.windows == "test" and dirty:
        err = "scripts/predict-check に未コミットの変更があります。コミットしてから test 窓を測ってください"
    if err:
        print(f"エラー: {err}", file=sys.stderr)
        return 2

    probs = pl.load_probs(args.probs)
    date, horses, fins = load_results(args.results)
    prob_odds, payout = load_odds(args.odds, args.wide_low)
    lines = [
        f"- 計測: git {version}{'（未コミットの変更あり）' if dirty else ''} / bootstrap {args.bootstrap} / seed {args.seed}",
        f"- 入力: 確率 `{os.path.basename(args.probs)}`（sha256 先頭 12 桁 {pl._file_sha(args.probs)}）・"
        f"確定オッズ `{os.path.basename(args.odds)}`（{pl._file_sha(args.odds)}）・着順 `{os.path.basename(args.results)}`"
        f"（{pl._file_sha(args.results)}）",
        f"- 窓: fit {args.dev_from}〜{args.dev_to}"
        + (f" / eval {args.test_from}〜{args.test_to}" if args.windows == "test" else "（eval は測らない）"),
        f"- 定義: 3連複の無投票 = {'除外して正規化' if args.trio_no_floor else '下限（売れた最小 π の半分）'}・"
        f"ワイドの π = {'下限オッズ' if args.wide_low else '中点オッズ'}・オッズは確定値のみ",
        "",
    ]
    for bt in bets:
        fr, fw = collect(bt, args.dev_from, args.dev_to, date, horses, fins, probs, prob_odds, payout, not args.trio_no_floor)
        er = ew = None
        if args.windows == "test":
            er, ew = collect(bt, args.test_from, args.test_to, date, horses, fins, probs, prob_odds, payout, True)
        body, _ = report_bet(bt, fr, fw, er, ew, args.bootstrap, args.seed)
        lines += body
    print("\n".join(lines))
    if args.ledger:
        pl.append_ledger(args.ledger, args.label, lines)
    return 0


if __name__ == "__main__":
    sys.exit(main())
