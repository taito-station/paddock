#!/usr/bin/env python3
"""独立確率モデル（#720）の特徴量。results と races だけから、市場情報を使わずに作る。

入力は `pl_features_extract.sql` の出力 TSV（列は ALLOWED_COLUMNS と完全一致。odds・popularity などの
市場情報が混ざったら読み込みで拒否する）。horse_past_runs は使わない（取得済みの馬に偏る生存バイアス）。
馬のキーは馬名（results.horse_id は半数ほどしか埋まっていない）。

as-of の規約（リーク防止）:
- 馬の履歴・騎手・調教師の成績は、そのレースの日付より前（同日の別レースも含めない）の結果だけで数える。
- 走破タイムの基準（コース × 馬場の平均・SD）は、そのレースの月の月初より前の完走だけで作る。
  これで各レースの特徴量は一度だけ決まり、walk-forward の学習窓ごとに作り直さなくてよい。

出走馬（status が finished / did_not_finish）だけを出力する。取消・除外は出走していないので出さない。
中止（did_not_finish）は着順なしで出力し、学習側（train_pl_topk.stage_winners）が分母に残す。

使い方（出力は gitignore 済みの scripts/harness/data/ に置く。DB 由来のデータを公開リポジトリに混ぜない）:
  python3 scripts/harness/pl_features.py scripts/harness/data/raw.tsv -o scripts/harness/data/feats.tsv
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import math
import sys
from collections import defaultdict
from datetime import date as _date

ALLOWED_COLUMNS = [
    "race_id", "date", "venue", "surface", "distance", "track_condition",
    "horse_num", "gate_num", "horse_name", "jockey", "trainer",
    "weight_carried", "horse_weight", "weight_change",
    "status", "finishing_position", "time_seconds",
]
_INT_COLS = {"distance", "horse_num", "gate_num", "horse_weight", "weight_change", "finishing_position"}
_FLOAT_COLS = {"weight_carried", "time_seconds"}
STARTERS = {"finished", "did_not_finish"}
STATUSES = STARTERS | {"scratched", "cancelled"}  # ResultStatus と同じ値域
HEAVY_GOING = {"稍重", "重", "不良"}
GOOD_GOING = {"良"}  # 良・重どちらでもない値（空＝未取得）は「馬場不明」として扱う（#745）

FEATURES = [
    "n_runs", "no_hist", "days_since", "form3", "last_rel", "dnf_rate",
    "behind3", "best_speed3", "mean_speed3", "no_speed",
    "surface_apt", "surface_share", "dist_apt", "dist_dir", "dist_gap", "venue_apt", "going_apt",
    "weight_carried", "horse_weight", "weight_change", "gate_frac",
    "jockey_win", "jockey_top3", "jockey_rides", "trainer_win", "trainer_top3",
]

NEUTRAL_REL = 0.5  # 相対着順（0 = 1 着・1 = 最下位）の中立値。履歴なし・1 頭立て
APT_M = 2.0  # 条件別成績を馬の全体成績へ縮約する擬似件数
JOCKEY_M = 30.0  # 騎手・調教師の率を事前分布へ縮約する擬似件数
WIN_PRIOR = 1.0 / 14  # 平均頭数ほどの勝率
TOP3_PRIOR = 3.0 / 14
PAR_MIN_N = 30  # 基準タイムに要る完走数（未満なら馬場を区別しないキーに落とし、それも未満なら使わない）
SPEED_CAP = 4.0  # 速度 z の絶対値の上限（大差の失速・記録誤りの外れ値を抑える）
BEHIND_CAP = 5.0  # 勝ち馬とのタイム差（秒 / 1000m）の上限（大差負けの外れ値を抑える）
BEHIND_IMPUTE = 1.0
DAYS_IMPUTE = 60


# ---------- 入力 ----------


def load_raw(path: str) -> list[dict]:
    """抽出 TSV を読む。ヘッダが許可リストと一致しなければ拒否（市場情報の混入を入口で止める）。"""
    with open(path, encoding="utf-8", newline="") as f:
        reader = csv.reader(f, delimiter="\t")
        header = [h.strip() for h in next(reader, [])]
        if sorted(header) != sorted(ALLOWED_COLUMNS):
            extra = sorted(set(header) - set(ALLOWED_COLUMNS))
            missing = sorted(set(ALLOWED_COLUMNS) - set(header))
            raise ValueError(f"{path}: 列が許可リストと一致しません（余分 {extra} / 不足 {missing}）")
        out = []
        for lineno, cells in enumerate(reader, start=2):
            if not cells:
                continue
            if len(cells) != len(header):
                raise ValueError(f"{path}:{lineno} 列数が {len(cells)}（ヘッダは {len(header)}）")
            r: dict = {}
            for name, cell in zip(header, cells):
                v = cell.strip()
                try:
                    if name in _INT_COLS:
                        r[name] = int(v) if v else None
                    elif name in _FLOAT_COLS:
                        r[name] = float(v) if v else None
                        # NaN/inf は基準タイムの累積や特徴量に黙って伝播するので入口で止める
                        if r[name] is not None and not math.isfinite(r[name]):
                            raise ValueError("有限でない")
                    else:
                        r[name] = v
                except ValueError as e:
                    raise ValueError(f"{path}:{lineno} {name} の値が不正です: {cell!r}（{e}）") from e
            try:
                _date.fromisoformat(r["date"])
            except ValueError as e:
                raise ValueError(f"{path}:{lineno} date が YYYY-MM-DD ではありません: {r['date']!r}") from e
            # 馬名は履歴のキー、race_id はレースのキー。空だと別の馬・レースが 1 つに合算される
            for key in ("race_id", "horse_name"):
                if not r[key]:
                    raise ValueError(f"{path}:{lineno} {key} が空です")
            if r["status"] not in STATUSES:
                raise ValueError(f"{path}:{lineno} status が未知の値です: {r['status']!r}")
            # 完走なのに着順が無い行（ラベル欠落）は中止と区別できないので入口で止める
            if r["status"] == "finished" and r["finishing_position"] is None:
                raise ValueError(f"{path}:{lineno} 完走（finished）なのに finishing_position が空です")
            out.append(r)
    return out


# ---------- 補助 ----------


def _going_heavy(cond: str) -> bool | None:
    """重馬場なら True・良なら False・馬場不明なら None（良として数えない・#745）。"""
    if cond in HEAVY_GOING:
        return True
    return False if cond in GOOD_GOING else None


def _month_start(d: str) -> str:
    return d[:7] + "-01"


def _days(a: str, b: str) -> int:
    return (_date.fromisoformat(b) - _date.fromisoformat(a)).days


def _mean(xs: list[float], default: float) -> float:
    return sum(xs) / len(xs) if xs else default


class _Par:
    """月初時点の基準タイム（コース × 馬場）。月初より前の完走を順に足し込み、月ごとに写しを取る。

    馬場不明の走は細かいキー（馬場別）を None にし、馬場を区別しない粗いキーにだけ入れる。
    """

    def __init__(self, finished: list[tuple[str, tuple, tuple, float]]) -> None:
        # (date, 細かいキー, 粗いキー, time)。細かいキーは None を含むので日付だけで並べる
        self._recs = sorted(finished, key=lambda rec: rec[0])
        self._snap: dict[str, dict] = {}

    def at(self, cutoff: str) -> dict:
        if cutoff not in self._snap:
            acc: dict = defaultdict(lambda: [0, 0.0, 0.0])
            for d, fine, coarse, t in self._recs:
                if d >= cutoff:
                    break
                for k in (fine, coarse):
                    if k is None:
                        continue
                    a = acc[k]
                    a[0] += 1
                    a[1] += t
                    a[2] += t * t
            self._snap[cutoff] = dict(acc)
        return self._snap[cutoff]

    @staticmethod
    def z(stats: dict, fine: tuple, coarse: tuple, t: float) -> float | None:
        """基準より速いほど正。件数不足・分散 0 なら None。"""
        for k in (fine, coarse):
            s = stats.get(k)
            if s and s[0] >= PAR_MIN_N:
                n, sm, sq = s
                mu = sm / n
                var = sq / n - mu * mu
                if var > 1e-9:
                    return min(max((mu - t) / math.sqrt(var), -SPEED_CAP), SPEED_CAP)
        return None


def _apt(overall: float, sub: list[float]) -> float:
    """全体成績 − 条件別成績（全体へ縮約）。相対着順は小さいほど良いので、正が得意。"""
    return overall - (sum(sub) + APT_M * overall) / (len(sub) + APT_M)


def _rate(k: int, n: int, prior: float) -> float:
    return (k + JOCKEY_M * prior) / (n + JOCKEY_M)


# ---------- 特徴量 ----------


def build_features(rows: list[dict]) -> list[dict]:
    """全レースの出走馬ごとの特徴量（as-of）。rows は load_raw の出力（市場列は読まない）。"""
    races: dict = defaultdict(list)
    for r in rows:
        races[r["race_id"]].append(r)
    order = sorted(races, key=lambda rid: (races[rid][0]["date"], rid))

    finished = []
    for rid in order:
        for r in races[rid]:
            if r["status"] == "finished" and r["time_seconds"] is not None:
                heavy = _going_heavy(r["track_condition"])
                coarse = (r["venue"], r["surface"], r["distance"])
                fine = None if heavy is None else coarse + (heavy,)
                finished.append((r["date"], fine, coarse, r["time_seconds"]))
    par = _Par(finished)

    hist: dict = defaultdict(list)  # 馬名 → 過去走の記録
    jockey: dict = defaultdict(lambda: [0, 0, 0])  # 騎乗・1 着・3 着内
    trainer: dict = defaultdict(lambda: [0, 0, 0])
    out = []
    i = 0
    while i < len(order):
        day = races[order[i]][0]["date"]
        j = i
        while j < len(order) and races[order[j]][0]["date"] == day:
            j += 1
        day_races = order[i:j]
        pending = []  # 当日の結果はその日の全レースを出し終えてから履歴に足す（同日の別レースを使わない）
        for rid in day_races:
            starters = [r for r in races[rid] if r["status"] in STARTERS]
            if not starters:
                continue
            stats = par.at(_month_start(day))
            max_gate = max(r["gate_num"] for r in starters)
            wc_mean = _mean([r["weight_carried"] for r in starters if r["weight_carried"] is not None], 55.0)
            for r in starters:
                out.append(_horse_features(r, hist[r["horse_name"]], stats, max_gate, wc_mean,
                                           jockey.get(r["jockey"]), trainer.get(r["trainer"])))
            pending.append((rid, starters))
        for rid, starters in pending:
            n = len(starters)
            winner = [r["time_seconds"] for r in starters if r["finishing_position"] == 1 and r["time_seconds"]]
            wtime = winner[0] if len(winner) == 1 else None
            for r in starters:
                pos = r["finishing_position"]
                dnf = r["status"] == "did_not_finish"
                if dnf or pos is None:
                    rel = 1.0
                elif n >= 2:
                    rel = (pos - 1) / (n - 1)
                else:
                    rel = NEUTRAL_REL
                behind = None
                if wtime is not None and r["time_seconds"] is not None and r["distance"]:
                    behind = min(max((r["time_seconds"] - wtime) / (r["distance"] / 1000), 0.0), BEHIND_CAP)
                heavy = _going_heavy(r["track_condition"])
                coarse = (r["venue"], r["surface"], r["distance"])
                fine = None if heavy is None else coarse + (heavy,)
                hist[r["horse_name"]].append({
                    "date": day, "venue": r["venue"], "surface": r["surface"], "distance": r["distance"],
                    "heavy": heavy, "rel": rel, "dnf": dnf, "behind": behind,
                    "time": r["time_seconds"] if r["status"] == "finished" else None, "fine": fine, "coarse": coarse,
                })
                for table, key in ((jockey, r["jockey"]), (trainer, r["trainer"])):
                    if key:
                        c = table[key]
                        c[0] += 1
                        c[1] += int(pos == 1)
                        c[2] += int(pos is not None and pos <= 3)
        i = j
    return out


def _horse_features(r: dict, h: list[dict], stats: dict, max_gate: int, wc_mean: float,
                    jc: list | None, tc: list | None) -> dict:
    n = len(h)
    last3 = h[-3:]
    rels = [x["rel"] for x in h]
    overall = _mean(rels, NEUTRAL_REL)
    speeds = [z for z in (_Par.z(stats, x["fine"], x["coarse"], x["time"]) for x in last3 if x["time"] is not None)
              if z is not None]
    behinds = [x["behind"] for x in last3 if x["behind"] is not None]
    heavy = _going_heavy(r["track_condition"])

    def apt(pred) -> float:
        return _apt(overall, [x["rel"] for x in h if pred(x)]) if n else 0.0

    same_surface = [x for x in h if x["surface"] == r["surface"]]
    mean_dist = _mean([x["distance"] for x in h], float(r["distance"]))
    jc = jc or [0, 0, 0]
    tc = tc or [0, 0, 0]
    f = {
        "n_runs": math.log1p(n),
        "no_hist": float(n == 0),
        "days_since": math.log1p(_days(h[-1]["date"], r["date"]) if n else DAYS_IMPUTE),
        "form3": _mean([x["rel"] for x in last3], NEUTRAL_REL),
        "last_rel": h[-1]["rel"] if n else NEUTRAL_REL,
        "dnf_rate": _mean([float(x["dnf"]) for x in h], 0.0),
        "behind3": _mean(behinds, BEHIND_IMPUTE),
        "best_speed3": max(speeds) if speeds else 0.0,
        "mean_speed3": _mean(speeds, 0.0),
        "no_speed": float(not speeds),
        "surface_apt": apt(lambda x: x["surface"] == r["surface"]),
        "surface_share": len(same_surface) / n if n else 0.0,
        "dist_apt": apt(lambda x: abs(x["distance"] - r["distance"]) <= 200),
        "dist_dir": (r["distance"] - mean_dist) / 1000,
        "dist_gap": abs(r["distance"] - mean_dist) / 1000,
        "venue_apt": apt(lambda x: x["venue"] == r["venue"]),
        # 当日の馬場が不明なら適性は中立（データなしと同じ）。過去走の馬場不明は良・重どちらにも入らない
        "going_apt": apt(lambda x: x["heavy"] == heavy) if heavy is not None else 0.0,
        "weight_carried": r["weight_carried"] if r["weight_carried"] is not None else wc_mean,
        "horse_weight": (r["horse_weight"] if r["horse_weight"] is not None else 470) / 100,
        "weight_change": (r["weight_change"] if r["weight_change"] is not None else 0) / 10,
        "gate_frac": (r["gate_num"] - 1) / (max_gate - 1) if max_gate > 1 else 0.5,
        "jockey_win": _rate(jc[1], jc[0], WIN_PRIOR),
        "jockey_top3": _rate(jc[2], jc[0], TOP3_PRIOR),
        "jockey_rides": math.log1p(jc[0]),
        "trainer_win": _rate(tc[1], tc[0], WIN_PRIOR),
        "trainer_top3": _rate(tc[2], tc[0], TOP3_PRIOR),
    }
    return {
        "race_id": r["race_id"], "date": r["date"], "horse_num": r["horse_num"], "horse_name": r["horse_name"],
        "finishing_position": r["finishing_position"], "status": r["status"], **f,
    }


# ---------- 出力 ----------

META_COLUMNS = ["race_id", "date", "horse_num", "horse_name", "finishing_position", "status"]


def write_features(path: str, feats: list[dict]) -> None:
    with open(path, "w", encoding="utf-8", newline="") as f:
        w = csv.writer(f, delimiter="\t", lineterminator="\n")
        w.writerow(META_COLUMNS + FEATURES)
        for o in feats:
            w.writerow([("" if o[c] is None else o[c]) for c in META_COLUMNS] + [repr(float(o[c])) for c in FEATURES])


def file_sha(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:12]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("raw", help="pl_features_extract.sql の出力 TSV")
    ap.add_argument("-o", "--out", required=True)
    args = ap.parse_args(argv)
    feats = build_features(load_raw(args.raw))
    write_features(args.out, feats)
    print(f"raw {args.raw} sha256 {file_sha(args.raw)} → {args.out} sha256 {file_sha(args.out)}（{len(feats)} 行）",
          file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
