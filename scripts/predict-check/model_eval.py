#!/usr/bin/env python3
"""モデル精度の評価の土俵（#802）: ◎ の的中・上位 5 頭の包含・較正を、市場と並べて日クラスタ bootstrap の 95% CI 付きで出す。

`analyze backtest --dump-features` の TSV（as-of・リーク無し）を 1 つ以上受け取り、先頭を **基準**、
以降を **候補** として同じレース集合で比べる。買い方（回収率・EV）は扱わない——モデル層は当たること
だけで評価する（product-goals.md 目標 1・#801）。買い方層の指標は #803。

使い方:
  python3 scripts/predict-check/model_eval.py base=work/bt_dump_<sha>.tsv [cand=work/bt_dump_<sha2>.tsv ...] \\
      [--eval-from 2026-01-01 --eval-to 2026-08-31 | --all] [--bootstrap 1000] [--seed 42] [--json out.json]

系統: market（単勝オッズの含意確率。win のみ）/ pure（model_*_pure）/ blended（model_*＝本番構成）。
候補 dump は `<label>:pure` / `<label>:blended` として足す。

指標（レース単位の寄与を日ごとに束ねて復元抽出＝日クラスタ bootstrap。系統間・dump 間の差は同じ再標本で対応あり）:
  - ◎（win 確率の最大。同値は馬番昇順＝ backtest.rs）の 1 着 / 2 着以内 / 3 着以内率
  - 3 着以内馬のうち系統の win 順位 ≤5 の頭数（平均・全員率）。pure/blended は show 順位版も。
    2・3 着の同着で 3 着以内が 4 頭以上のときは、全員が入って初めて「3 頭すべて」、平均頭数は 3 で頭打ち
  - win/place/show の Brier・LogLoss（market は win のみ）と等幅 10 ビン reliability（metrics.rs と同じ切り方。
    帯ごとの実現率にも CI を付ける）
採否（候補 − 基準）: show_hit の CI が正、または win_logloss の CI が負で、かつ悪化側で 0 を跨がない主指標が無ければ
「採用候補」。悪化側で 0 を跨がない主指標が 1 つでもあれば「悪化」。それ以外は「判定不能」（knowledge/backtest.md 決定ログ #802）。
市場との差（`− market`）は参考で、採否の判定は付けない（基準は候補 dump と基準 dump の比較に定義されたもの）。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from collections import OrderedDict
from dataclasses import dataclass
from datetime import date as _date

import numpy as np

import prob_eval as pe

# 凍結 eval 窓（knowledge/backtest.md「fit/eval 窓の凍結」・#703）。全期間は --all で明示する。
EVAL_FROM_DEFAULT = "2026-01-01"
EVAL_TO_DEFAULT = "2026-08-31"

TOP_N = 5
TARGETS = {"win": 1, "place": 2, "show": 3}
BINS = 10
LOG_LOSS_EPS = 1e-15  # metrics.rs と同じクランプ幅
# 帯ごとの実現率の CI を出す最低件数と、再標本で空だった回の許容割合（疎ビンの条件付き CI を出さない）。
RELIABILITY_CI_MIN_COUNT = 10
RELIABILITY_CI_MAX_EMPTY_FRAC = 0.05
SYSTEMS_BASE = ("market", "pure", "blended")

# 採否の主指標と改善方向（+1: 大きいほど良い / −1: 小さいほど良い）。
PRIMARY = {"show_hit": +1, "win_hit": +1, "top5_win_all3": +1, "win_logloss": -1}
# どちらかの CI が改善側で 0 を跨がなければ「採用候補」の決め手になる指標。
DECISIVE = ("show_hit", "win_logloss")


# ---------- 入力 ----------


def parse_dump_args(items: list[str]) -> list[tuple[str, str]]:
    """`label=path` の列を (label, path) に。ラベル無し・重複は拒否する（出力の列名が曖昧になる）。"""
    out: list[tuple[str, str]] = []
    seen: set[str] = set()
    for item in items:
        if "=" not in item:
            raise ValueError(f"dump は label=path の形で指定してください: {item!r}")
        label, path = item.split("=", 1)
        if not label or label in seen or ":" in label:
            raise ValueError(f"dump のラベルが空・重複・':' 入りです: {label!r}")
        seen.add(label)
        out.append((label, path))
    return out


def filter_window(rows: list[dict], eval_from: str | None, eval_to: str | None) -> list[dict]:
    """date（YYYY-MM-DD・辞書順＝時系列順）で窓を切る。両端とも含む。None/None は全期間。"""
    return [
        r
        for r in rows
        if (eval_from is None or r["date"] >= eval_from) and (eval_to is None or r["date"] <= eval_to)
    ]


def label(row: dict, thresh: int) -> float:
    """着順ラベル。着順なし（中止・除外等）は不的中として 0（evaluate.rs・prob_eval と同じ扱い）。"""
    fin = row.get("finishing_position")
    return 1.0 if fin is not None and fin <= thresh else 0.0


def _has_horse_num(rows: list[dict]) -> bool:
    return all(r.get("horse_num") is not None for r in rows)


def eligible_races(rows: list[dict]) -> "OrderedDict[str, list[dict]]":
    """勝者が一意 かつ 全馬にオッズがあり かつ 全馬に馬番があるレース（3 系統を同じ母集合で比べる）。"""
    usable = pe.build_race_table(rows).winner_races()
    return OrderedDict(
        (rid, rr) for rid, rr in usable.items() if pe.market_probs(rr) is not None and _has_horse_num(rr)
    )


def exclusion_counts(rows: list[dict]) -> dict:
    table = pe.build_race_table(rows)
    usable = table.winner_races()
    odds_incomplete = sum(1 for rr in usable.values() if pe.market_probs(rr) is None)
    horse_num_missing = sum(
        1 for rr in usable.values() if pe.market_probs(rr) is not None and not _has_horse_num(rr)
    )
    return {
        "dead_heat": int(table.excluded["dead_heat"]),
        "no_winner": int(table.excluded["no_winner"]),
        "odds_incomplete": odds_incomplete,
        "horse_num_missing": horse_num_missing,
    }


def align_race_sets(
    tables: dict[str, "OrderedDict[str, list[dict]]"],
) -> tuple[list[str], dict[str, int]]:
    """複数 dump のレース集合を積集合に揃える。順序は先頭（基準）の dump に従う。dropped は dump ごとの脱落数。"""
    labels = list(tables)
    common = set(tables[labels[0]])
    for lb in labels[1:]:
        common &= set(tables[lb])
    ordered = [rid for rid in tables[labels[0]] if rid in common]
    dropped = {lb: len(tables[lb]) - len(ordered) for lb in labels}
    return ordered, dropped


# ---------- 系統と ◎ ----------


def system_probs(rows: list[dict], system: str) -> dict[str, list[float] | None]:
    """系統ごとの (win, place, show) 確率。market は win のみ（place/show は None）。"""
    if system == "market":
        q = pe.market_probs(rows)
        if q is None:
            raise ValueError("market 系統はオッズ完全なレースにだけ使えます（eligible_races で揃えること）")
        return {"win": q, "place": None, "show": None}
    suffix = "_pure" if system == "pure" else ""
    return {t: [float(r[f"model_{t}{suffix}"]) for r in rows] for t in TARGETS}


def rank_order(rows: list[dict], probs: list[float]) -> list[int]:
    """確率降順・同値は馬番昇順の添字列（backtest.rs の ◎ 規則をそのまま順位に広げたもの）。

    馬番が無い行（手編集 dump。正規 dump は u32 なので出ない）は同値のとき末尾に回し、比較で落ちないようにする。
    """
    return sorted(
        range(len(rows)),
        key=lambda i: (-probs[i], rows[i]["horse_num"] is None, rows[i]["horse_num"] or 0),
    )


def top_pick(rows: list[dict], probs: list[float]) -> int:
    return rank_order(rows, probs)[0]


def bin_index(p: np.ndarray) -> np.ndarray:
    """等幅ビンの添字。metrics.rs と同じ `p / width` で割る（`p * BINS` だと 0.3 等の境界値でビンが 1 つずれる）。"""
    width = 1.0 / BINS
    return np.minimum((np.clip(p, 0.0, 1.0) / width).astype(int), BINS - 1)


@dataclass
class RaceStat:
    """1 レースの寄与。bootstrap は日ごとにこれを束ねて足し合わせるだけで済むように、和の形で持つ。"""

    date: str
    hits: dict[str, int]  # ◎ の win/place/show 的中（0/1）
    placed_n: int  # 3 着以内の頭数（2・3 着の同着で 4 以上になりうる）
    incl_win: int  # 3 着以内馬のうち win 順位 ≤ TOP_N の頭数
    incl_show: int | None  # 同 show 順位（market は None）
    cal: dict[str, tuple[float, float, int]]  # target → (Σ(p−y)², Σ logloss, n)
    bins: dict[str, np.ndarray]  # target → (BINS, 3) = [Σp, hits, count]


def per_race_stats(rows: list[dict], probs: dict[str, list[float] | None]) -> RaceStat:
    win = probs["win"]
    assert win is not None
    pick = top_pick(rows, win)
    pos = rows[pick].get("finishing_position")
    hits = {t: int(pos is not None and pos <= th) for t, th in TARGETS.items()}

    placed = [i for i, r in enumerate(rows) if label(r, 3) == 1.0]

    def _incl(p: list[float]) -> int:
        top = set(rank_order(rows, p)[:TOP_N])
        return sum(1 for i in placed if i in top)

    incl_win = _incl(win)
    incl_show = _incl(probs["show"]) if probs["show"] is not None else None

    cal: dict[str, tuple[float, float, int]] = {}
    bins: dict[str, np.ndarray] = {}
    for t, th in TARGETS.items():
        p = probs[t]
        if p is None:
            continue
        pa = np.asarray(p, dtype=float)
        y = np.array([label(r, th) for r in rows])
        pc = np.clip(pa, LOG_LOSS_EPS, 1.0 - LOG_LOSS_EPS)
        ll = -(y * np.log(pc) + (1.0 - y) * np.log(1.0 - pc))
        cal[t] = (float(np.sum((pa - y) ** 2)), float(np.sum(ll)), len(rows))
        b = np.zeros((BINS, 3))
        idx = bin_index(pa)
        np.add.at(b[:, 0], idx, pa)
        np.add.at(b[:, 1], idx, y)
        np.add.at(b[:, 2], idx, 1.0)
        bins[t] = b
    return RaceStat(
        date=rows[0]["date"],
        hits=hits,
        placed_n=len(placed),
        incl_win=incl_win,
        incl_show=incl_show,
        cal=cal,
        bins=bins,
    )


# ---------- 集計 ----------


class _Arrays:
    """RaceStat の列を numpy に並べたもの（bootstrap で添字抽出して和を取るため）。"""

    def __init__(self, stats: list[RaceStat]):
        self.n = len(stats)
        self.dates = np.array([s.date for s in stats])
        self.hits = np.array([[s.hits[t] for t in TARGETS] for s in stats], dtype=float).reshape(self.n, 3)
        self.placed_n = np.array([s.placed_n for s in stats], dtype=float)
        self.incl_win = np.array([s.incl_win for s in stats], dtype=float)
        has_show = all(s.incl_show is not None for s in stats)
        self.incl_show = np.array([s.incl_show for s in stats], dtype=float) if has_show else None
        self.cal = {
            t: np.array([s.cal[t] for s in stats], dtype=float)
            for t in TARGETS
            if all(t in s.cal for s in stats)
        }
        self.bins = {t: np.stack([s.bins[t] for s in stats]) for t in self.cal}  # (n, BINS, 3)

    def _incl_metrics(self, incl: np.ndarray, placed_n: np.ndarray, key: str, m: dict[str, float]) -> None:
        m[f"{key}_mean"] = float(np.minimum(incl, 3).mean())
        # 「3 頭すべて」は 3 着以内が 3 頭以上そろっているレースでだけ成立する（着順欠落で 1〜2 頭しか無い
        # レースを「全員包含」と数えて指標を押し上げない）。
        m[f"{key}_all3"] = float(((incl >= placed_n) & (placed_n >= 3)).mean())

    def metrics(self, idx: np.ndarray | None = None) -> dict[str, float]:
        sel = slice(None) if idx is None else idx
        n = self.n if idx is None else len(idx)
        if n == 0:
            raise ValueError("対象レースが 0 件です")
        hits = self.hits[sel]
        m: dict[str, float] = {"races": float(n)}
        for j, t in enumerate(TARGETS):
            m[f"{t}_hit"] = float(hits[:, j].mean())
        self._incl_metrics(self.incl_win[sel], self.placed_n[sel], "top5_win", m)
        if self.incl_show is not None:
            self._incl_metrics(self.incl_show[sel], self.placed_n[sel], "top5_show", m)
        for t, arr in self.cal.items():
            a = arr[sel]
            horses = a[:, 2].sum()
            m[f"{t}_brier"] = float(a[:, 0].sum() / horses)
            m[f"{t}_logloss"] = float(a[:, 1].sum() / horses)
        return m

    def bin_rates(self, target: str, idx: np.ndarray | None = None) -> np.ndarray:
        """帯ごとの実現率（空ビンは nan）。bootstrap の再標本でも同じ経路で出す。"""
        b = self.bins[target] if idx is None else self.bins[target][idx]
        tot = b.sum(axis=0)
        with np.errstate(invalid="ignore", divide="ignore"):
            return np.where(tot[:, 2] > 0, tot[:, 1] / tot[:, 2], np.nan)


def aggregate(stats: list[RaceStat]) -> dict[str, float]:
    m = _Arrays(stats).metrics()
    m["races"] = int(m["races"])
    return m


def reliability_bins(
    stats: list[RaceStat], target: str, rate_samples: np.ndarray | None = None
) -> list[dict]:
    """等幅 10 ビンの reliability（metrics.rs と同じ切り方）。空ビンも count=0 で返す。

    rate_samples（bootstrap_samples の `<target>_bin_rate`・(n_boot, BINS)）を渡すと帯ごとの実現率に CI を付ける。
    疎なビンは再標本で空になる回があり、その回を除いた分位は「非空の再標本に条件付けした CI」になる。
    空だった回の割合を `rate_ci_empty_frac` に出し、件数 10 未満か空率 5% 超のビンは CI を None にする。
    """
    arrays = _Arrays(stats)
    if target not in arrays.bins:
        return []
    tot = arrays.bins[target].sum(axis=0)
    out = []
    for k in range(BINS):
        cnt = int(tot[k, 2])
        item = {
            "bin": f"[{k / BINS:.1f},{(k + 1) / BINS:.1f})" if k < BINS - 1 else f"[{k / BINS:.1f},1.0]",
            "count": cnt,
            "mean_pred": float(tot[k, 0] / cnt) if cnt else None,
            "rate": float(tot[k, 1] / cnt) if cnt else None,
        }
        if rate_samples is not None and cnt:
            col = rate_samples[:, k]
            empty_frac = float(np.isnan(col).mean())
            col = col[~np.isnan(col)]
            item["rate_ci_empty_frac"] = empty_frac
            sparse = cnt < RELIABILITY_CI_MIN_COUNT or empty_frac > RELIABILITY_CI_MAX_EMPTY_FRAC
            item["rate_ci"] = (
                None if sparse or not len(col) else [float(np.quantile(col, 0.025)), float(np.quantile(col, 0.975))]
            )
        out.append(item)
    return out


def bootstrap_samples(
    stats_by_system: dict[str, list[RaceStat]], n_boot: int, seed: int
) -> dict[str, dict[str, np.ndarray]]:
    """日クラスタ bootstrap の再標本値。日付を復元抽出し、その日の全レースを採る。

    全系統が同じレース集合（同長・同順・同日付）であることを要求する——そうでないと対応あり Δ が無意味になる。
    返り値は system → metric → (n_boot,) の配列（帯ごとの実現率は `<target>_bin_rate` → (n_boot, BINS)）。
    全系統で同じ抽出を使う（対応あり）。
    """
    if n_boot < 1:
        raise ValueError("bootstrap の回数は 1 以上にしてください")
    arrays = {s: _Arrays(st) for s, st in stats_by_system.items()}
    first = next(iter(arrays.values()))
    for a in arrays.values():
        if a.n != first.n or not np.array_equal(a.dates, first.dates):
            raise ValueError("bootstrap は全系統が同じレース集合（同長・同順）であることが前提です")
    days = sorted(set(first.dates.tolist()))
    day_idx = {d: np.flatnonzero(first.dates == d) for d in days}
    rng = np.random.default_rng(seed)
    out: dict[str, dict[str, list]] = {s: {} for s in arrays}
    for _ in range(n_boot):
        picked = rng.integers(0, len(days), size=len(days))
        idx = np.concatenate([day_idx[days[k]] for k in picked])
        for s, a in arrays.items():
            for k, v in a.metrics(idx).items():
                out[s].setdefault(k, []).append(v)
            for t in a.bins:
                out[s].setdefault(f"{t}_bin_rate", []).append(a.bin_rates(t, idx))
    return {s: {k: np.array(v) for k, v in d.items()} for s, d in out.items()}


def ci_from_samples(samples: dict[str, dict[str, np.ndarray]]) -> dict[str, dict[str, tuple[float, float]]]:
    """各系統・各スカラー指標の 95% CI（2.5 / 97.5 分位）。帯ごとの実現率と races は含めない。"""
    return {
        s: {
            k: (float(np.quantile(v, 0.025)), float(np.quantile(v, 0.975)))
            for k, v in d.items()
            if k != "races" and v.ndim == 1
        }
        for s, d in samples.items()
    }


def day_bootstrap(
    stats_by_system: dict[str, list[RaceStat]], n_boot: int = 1000, seed: int = 42
) -> dict[str, dict[str, tuple[float, float]]]:
    return ci_from_samples(bootstrap_samples(stats_by_system, n_boot, seed))


def delta_ci(
    stats_by_system: dict[str, list[RaceStat]],
    cand: str,
    base: str,
    n_boot: int = 1000,
    seed: int = 42,
    samples: dict[str, dict[str, np.ndarray]] | None = None,
) -> dict[str, tuple[float, float, float]]:
    """候補 − 基準 の主指標の Δ（点推定, CI 下限, CI 上限）。同じ再標本で対応ありに取る。"""
    if samples is None:
        samples = bootstrap_samples({cand: stats_by_system[cand], base: stats_by_system[base]}, n_boot, seed)
    point_c = aggregate(stats_by_system[cand])
    point_b = aggregate(stats_by_system[base])
    out: dict[str, tuple[float, float, float]] = {}
    for k in PRIMARY:
        if k not in point_c or k not in point_b:
            continue
        d = samples[cand][k] - samples[base][k]
        out[k] = (point_c[k] - point_b[k], float(np.quantile(d, 0.025)), float(np.quantile(d, 0.975)))
    return out


def judge(delta: dict[str, tuple[float, float, float]]) -> str:
    """採否の機械判定（knowledge/backtest.md 決定ログ #802）。CI が 0 を含む指標は「差が言えない」として扱う。"""
    worse = False
    decisive = False
    for k, (_, lo, hi) in delta.items():
        direction = PRIMARY[k]
        if (direction > 0 and hi < 0) or (direction < 0 and lo > 0):
            worse = True
        if k in DECISIVE and ((direction > 0 and lo > 0) or (direction < 0 and hi < 0)):
            decisive = True
    if worse:
        return "悪化"
    return "採用候補" if decisive else "判定不能"


def fidelity(rows: list[dict]) -> dict[str, float]:
    """忠実性サニティ: 窓・母集合で絞る前の dump 全体で、blended の ◎ 的中率・win Brier・win LogLoss を analyze backtest と同じ定義で出す。

    analyze backtest は全評価レース（勝者なし・オッズ欠落を含む）を母集合にし、◎ の着順なしは不的中。
    ここでの数字が `analyze backtest` の win_hit_rate / show_hit_rate / 単勝 Brier / LogLoss と一致すれば dump と評価器は整合している。
    """
    races = pe.build_race_table(rows).races
    win_hits = show_hits = 0
    for rr in races.values():
        pick = top_pick(rr, [float(r["model_win"]) for r in rr])
        pos = rr[pick].get("finishing_position")
        win_hits += int(pos == 1)
        show_hits += int(pos is not None and pos <= 3)
    p = np.array([float(r["model_win"]) for r in rows])
    y = np.array([label(r, 1) for r in rows])
    pc = np.clip(p, LOG_LOSS_EPS, 1.0 - LOG_LOSS_EPS)
    n = len(races)
    return {
        "races": n,
        "win_hit": win_hits / n if n else float("nan"),
        "show_hit": show_hits / n if n else float("nan"),
        "win_brier": float(np.mean((p - y) ** 2)) if len(rows) else float("nan"),
        "win_logloss": float(np.mean(-(y * np.log(pc) + (1.0 - y) * np.log(1.0 - pc)))) if len(rows) else float("nan"),
    }


def file_sha256(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:12]


# ---------- レポート ----------


def _fmt_ci(point: float, ci: tuple[float, float] | None, pct: bool) -> str:
    if pct:
        s = f"{point * 100:5.1f}%"
        return s if ci is None else f"{s} [{ci[0] * 100:5.1f},{ci[1] * 100:5.1f}]"
    s = f"{point:.4f}"
    return s if ci is None else f"{s} [{ci[0]:.4f},{ci[1]:.4f}]"


def _print_systems(points: dict[str, dict], cis: dict[str, dict]) -> None:
    cols = [
        ("win_hit", "◎単勝", True),
        ("place_hit", "◎連対", True),
        ("show_hit", "◎複勝", True),
        ("top5_win_mean", "top5包含(平均頭数・3で頭打ち)", False),
        ("top5_win_all3", "top5包含(3頭すべて)", True),
        ("win_brier", "win Brier", False),
        ("win_logloss", "win LogLoss", False),
        ("top5_show_all3", "top5包含(show順位・3頭すべて)", True),
        ("show_brier", "show Brier", False),
        ("show_logloss", "show LogLoss", False),
    ]
    for key, title, pct in cols:
        rows_ = [(s, m) for s, m in points.items() if key in m]
        if rows_:
            print(f"  {title}:")
            for s, m in rows_:
                print(f"    {s:16} {_fmt_ci(m[key], cis.get(s, {}).get(key), pct)}")


def _print_delta(cand: str, base: str, d: dict[str, tuple[float, float, float]], verdict: str | None) -> None:
    print(f"  {cand} − {base}: {verdict if verdict else '（参考・採否の判定なし）'}")
    for k, (point, lo, hi) in d.items():
        pct = not k.endswith("logloss")
        f = (lambda x: f"{x * 100:+.1f}pt") if pct else (lambda x: f"{x:+.4f}")
        print(f"    {k:16} {f(point)} [{f(lo)}, {f(hi)}]")


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dumps", nargs="+", help="label=path の列。先頭が基準、以降が候補")
    ap.add_argument("--eval-from", default=None, help=f"eval 窓の開始日（既定 {EVAL_FROM_DEFAULT}・含む）")
    ap.add_argument("--eval-to", default=None, help=f"eval 窓の終端日（既定 {EVAL_TO_DEFAULT}・含む）")
    ap.add_argument("--all", action="store_true", help="窓で切らず dump 全期間を使う（採否判断には使わない。--eval-from/--eval-to と併用不可）")
    ap.add_argument("--bootstrap", type=int, default=1000, help="日クラスタ bootstrap の回数（既定 1000・1 以上）")
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--json", default=None, help="結果を JSON で書き出すパス（dump と同じパス・ディレクトリは不可）")
    args = ap.parse_args(argv)

    try:
        dumps = parse_dump_args(args.dumps)
    except ValueError as e:
        ap.error(str(e))
    if args.bootstrap < 1:
        ap.error("--bootstrap は 1 以上にしてください")
    if args.seed < 0:
        ap.error("--seed は 0 以上にしてください")
    for _, path in dumps:
        if not os.path.isfile(path):
            ap.error(f"dump が見つかりません: {path}")
    if args.all:
        if args.eval_from is not None or args.eval_to is not None:
            ap.error("--all と --eval-from/--eval-to は併用できません")
        eval_from = eval_to = None
    else:
        eval_from = args.eval_from or EVAL_FROM_DEFAULT
        eval_to = args.eval_to or EVAL_TO_DEFAULT
        # YYYY-MM-DD 以外は辞書順比較で黙って誤窓になる（fromisoformat は `20260101` も受理するので往復で絞る）。
        for name, v in (("--eval-from", eval_from), ("--eval-to", eval_to)):
            try:
                ok = _date.fromisoformat(v).isoformat() == v
            except ValueError:
                ok = False
            if not ok:
                ap.error(f"{name} は YYYY-MM-DD 形式で指定してください: {v!r}")
        if eval_from > eval_to:
            ap.error(f"--eval-from ({eval_from}) は --eval-to ({eval_to}) 以前にしてください")
    if args.json:
        if os.path.isdir(args.json):
            ap.error(f"--json にディレクトリは指定できません: {args.json}")
        for _, path in dumps:
            if os.path.exists(args.json) and os.path.samefile(args.json, path):
                ap.error(f"--json が dump と同じパスです（dump が上書きされます）: {args.json}")
        # 書けない先は全計算の後に初めて分かるので、前段で親ディレクトリの存在と書き込み権限を見る。
        parent = os.path.dirname(os.path.abspath(args.json))
        if not os.path.isdir(parent) or not os.access(parent, os.W_OK):
            ap.error(f"--json の書き出し先に書けません: {args.json}")

    base_label = dumps[0][0]
    loaded: dict[str, list[dict]] = {}
    tables: dict[str, OrderedDict] = {}
    fid: dict[str, dict] = {}
    provenance: list[dict] = []
    for lb, path in dumps:
        # 来歴（ハッシュ・サイズ）は読む直前に取る。bootstrap の後に開き直すと、同名 dump を再生成する
        # 並走セッションがいたとき評価に使った中身と食い違う。
        provenance.append({"label": lb, "path": path, "sha256_12": file_sha256(path), "bytes": os.path.getsize(path)})
        rows = pe.load_dump(path)
        fid[lb] = fidelity(rows)
        rows = filter_window(rows, eval_from, eval_to)
        loaded[lb] = rows
        tables[lb] = eligible_races(rows)
    common, dropped = align_race_sets(tables)
    if not common:
        ap.error("全 dump に共通するレースがありません（窓・dump の取り違えを確認）")

    # 系統ごとの RaceStat（基準: market/pure/blended、候補: <label>:pure / <label>:blended）
    stats: dict[str, list[RaceStat]] = {}
    for s in SYSTEMS_BASE:
        stats[s] = [per_race_stats(tables[base_label][rid], system_probs(tables[base_label][rid], s)) for rid in common]
    for lb, _ in dumps[1:]:
        for s in ("pure", "blended"):
            stats[f"{lb}:{s}"] = [per_race_stats(tables[lb][rid], system_probs(tables[lb][rid], s)) for rid in common]

    points = {s: aggregate(st) for s, st in stats.items()}
    samples = bootstrap_samples(stats, args.bootstrap, args.seed)
    cis = ci_from_samples(samples)

    window = "全期間" if eval_from is None else f"{eval_from}〜{eval_to}"
    excl = exclusion_counts(loaded[base_label])
    days = len({tables[base_label][rid][0]["date"] for rid in common})
    print(f"== モデル精度の土俵（#802） 窓: {window} / bootstrap {args.bootstrap} 回（日クラスタ・seed {args.seed}） ==")
    print(
        f"母集合: レース {len(common)}（{days} 日・基準 {base_label} の除外: 同着 {excl['dead_heat']} /"
        f" 勝者なし {excl['no_winner']} / オッズ不完全 {excl['odds_incomplete']} / 馬番欠落 {excl['horse_num_missing']}）"
        + ("" if len(dumps) == 1 else f" / 積集合で脱落 {dropped}")
    )
    for lb, f in fid.items():
        print(
            f"忠実性（{lb}・dump 全期間 {f['races']}R）: blended ◎単勝 {f['win_hit'] * 100:.1f}% / ◎複勝 {f['show_hit'] * 100:.1f}%"
            f" / win Brier {f['win_brier']:.6f} / win LogLoss {f['win_logloss']:.6f}（analyze backtest の出力と一致すること）"
        )
    print("\n指標（点推定 [95% CI]）:")
    _print_systems(points, cis)

    print("\nwin reliability（等幅 10 ビン・基準 dump。件数 平均予測→実測率 [実測率の 95% CI]）:")
    rel = {s: {t: reliability_bins(stats[s], t, samples[s].get(f"{t}_bin_rate")) for t in TARGETS if t in stats[s][0].cal} for s in stats}
    print(f"  {'bin':12}" + "".join(f" {s:>34}" for s in SYSTEMS_BASE))
    for k in range(BINS):
        cells = []
        for s in SYSTEMS_BASE:
            b = rel[s]["win"][k]
            if b["count"]:
                ci = b.get("rate_ci")
                ci_s = f" [{ci[0]:.3f},{ci[1]:.3f}]" if ci else ""
                cells.append(f"{b['count']:6d} {b['mean_pred']:.3f}→{b['rate']:.3f}{ci_s}")
            else:
                cells.append(f"{0:6d}      —")
        print(f"  {rel['market']['win'][k]['bin']:12}" + "".join(f" {c:>34}" for c in cells))

    # 市場との差は参考（採否の基準は候補 dump − 基準 dump に定義されたもの）。
    market_ref: dict[str, dict] = {}
    print("\n市場との差（参考・対応あり Δ [95% CI]・採否の判定なし）:")
    for s in [x for x in stats if x != "market"]:
        d = delta_ci(stats, s, "market", samples=samples)
        market_ref[s] = {k: list(v) for k, v in d.items()}
        _print_delta(s, "market", d, None)

    judgments: dict[str, dict] = {}
    print("\n採否（候補 − 基準・主指標の Δ [95% CI]）:")
    if len(dumps) == 1:
        print("  候補 dump なし（基準のみ）。")
    for lb, _ in dumps[1:]:
        for s in ("pure", "blended"):
            cand = f"{lb}:{s}"
            d = delta_ci(stats, cand, s, samples=samples)
            judgments[cand] = {"base": s, "delta": {k: list(v) for k, v in d.items()}, "verdict": judge(d)}
            _print_delta(cand, s, d, judgments[cand]["verdict"])

    if args.json:
        payload = {
            "args": {"dumps": dumps, "eval_from": eval_from, "eval_to": eval_to, "bootstrap": args.bootstrap, "seed": args.seed},
            "dumps": provenance,
            "population": {"races": len(common), "days": days, "excluded": excl, "dropped": dropped},
            "fidelity": fid,
            "systems": {s: {"point": points[s], "ci": {k: list(v) for k, v in cis[s].items()}} for s in stats},
            "reliability": rel,
            "market_reference": market_ref,
            "judgments": judgments,
        }
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump(payload, f, ensure_ascii=False, indent=2)
        print(f"\nJSON: {args.json}")


if __name__ == "__main__":
    main()
