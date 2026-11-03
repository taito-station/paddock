#!/usr/bin/env python3
"""朝〜直前のオッズで市場ブレンドの α を判定する（#218）。

#284 は確定オッズで α=0（市場そのもの）が本番の α=0.2 を有意に上回ったが、本番が α を当てるのは
朝〜直前のオッズなので定数は保留した。本スクリプトは `race_odds_snapshots` の単勝で時点ごとに市場を
作り直し、本番構成の α=0.2 ブレンドを同じ時点のオッズで厳密に再構成して、市場と比べる。

判定規則（測る前に固定・issue #218 の 2026-11-03 コメント）:
  - 時点は **朝**（当日 JST 0 時以降・発走前で、全馬の単勝がそろう最初のスナップショット）。
  - 基準 = 市場（その時点の単勝の含意確率・α=0）、候補 = α=0.2・γ=1.25 のブレンド。
  - model_eval の採否基準（backtest.md 決定ログ #802）で「採用候補」なら α=0.2 維持、
    「判定不能」「悪化」なら α=0 に倒す。T-40 と直前は参考（判定に使わない）。

使い方:
  python3 scripts/predict-check/live_alpha_eval.py work/bt_dump_<sha>.tsv [--bootstrap 1000] [--seed 42] [--json out.json]
DB は SELECT だけ（`zure_sign_probe.fetch_snapshots_with_cards`）。
"""

from __future__ import annotations

import argparse
import json
import os
from collections import Counter, OrderedDict

import numpy as np

import mag_sweep as ms
import model_eval as me
import prob_eval as pe
import zure_sign_probe as zsp

DUMP_ALPHA = ms.DUMP_ALPHA  # 0.2（本番）
DUMP_GAMMA = ms.GAMMA0  # 1.25（本番の win_power）
T40_MIN = 40
POINTS = ("morning", "t40", "last")
POINT_LABEL = {"morning": "朝（判定）", "t40": "T-40（参考）", "last": "直前（参考）"}
DECISION_POINT = "morning"
MAX_FIDELITY_EXCLUDED_FRAC = 0.01  # #284 と同じ停止ガード


def select_points(entry: dict, horses: set[int]) -> dict | None:
    """1 レースのスナップショットから 朝 / T-40 / 直前 のオッズ {馬番: odds} を選ぶ。

    使うのは JST 日付が開催日と一致し、発走前（取得の JST 分 ≤ 発走の分）で、dump の全馬に単勝がそろう
    「完全な」スナップショットだけ。発走時刻が無いレースは None。該当なしの時点は None。
    時点の定義は事前コミット（issue #218）のもので、本番の近似:
    - 朝: 本番の朝の選定（`FirstWinSince(JST 0 時)`）は単勝が 1 行でもあれば採り、発走 40 分前以降なら朝と
      見なさない（TooLate）。ここでは全馬そろいを要求し、発走前なら時刻を問わない。差は `morning_disclosure` で開示する。
    - T-40: 発走 40 分前以降で最初（境界を含む）。本番の `FirstCompleteSince` は初回スイープ時刻以降で全券種が
      そろう最初のスナップショットで、起点も完全性の定義も違う。
    """
    post = entry.get("post_min")
    if post is None:
        return None
    valid = []
    for ts in sorted(entry["snaps"]):
        d, mn = zsp.jst_date_min(ts)
        odds = entry["snaps"][ts]
        if d == entry["date"] and mn <= post and horses <= set(odds):
            valid.append((mn, {h: odds[h] for h in horses}))
    if not valid:
        return {p: None for p in POINTS}
    t40 = next((o for mn, o in valid if mn >= post - T40_MIN), None)
    return {"morning": valid[0][1], "t40": t40, "last": valid[-1][1]}


def morning_disclosure(entry: dict, horses: set[int]) -> dict[str, bool]:
    """朝の時点が本番の朝の選定と食い違うかを開示する（判定には使わない）。

    late: 選んだ朝のスナップショットが発走 40 分前以降（本番なら TooLate で朝の選定が無い）。
    differs: その日の最初の単勝スナップショット（全馬そろいを問わない＝本番の FirstWinSince）と別の取得。
    """
    post = entry["post_min"]
    same_day = [ts for ts in sorted(entry["snaps"]) if zsp.jst_date_min(ts)[0] == entry["date"] and zsp.jst_date_min(ts)[1] <= post]
    complete = [ts for ts in same_day if horses <= set(entry["snaps"][ts])]
    if not complete:
        return {"late": False, "differs": False}
    chosen = complete[0]
    return {"late": zsp.jst_date_min(chosen)[1] >= post - T40_MIN, "differs": chosen != same_day[0]}


def check_fidelity(n_unexplained: int, n_in_scope: int) -> None:
    """説明できない再構成の不一致が 1% を超えたら停止する（#218 事前コミットの修正）。ちょうど 1% は通す。"""
    frac = n_unexplained / n_in_scope if n_in_scope else 0.0
    if frac > MAX_FIDELITY_EXCLUDED_FRAC:
        raise SystemExit(
            f"説明できない再構成の不一致が {n_unexplained}/{n_in_scope}（{frac:.1%}）で 1% を超えたので停止します"
        )


def reconstruct_with_scratched(rows: list[dict], race_odds_win: dict[int, float]) -> np.ndarray:
    """本番 blend の再現（取消馬込み）: `race_odds` の単勝のうち dump に出てこない馬（取消馬）の implied も
    市場確率の正規化の分母に入れる。dump の `model_win` と一致すれば、不一致の原因は取消馬だと説明できる。
    """
    field = {r["horse_num"] for r in rows}
    u = np.array([float(r["model_win_pure"]) for r in rows])
    est = np.power(np.maximum(u, 0.0), 1.0 / DUMP_GAMMA)
    e_hat = est / est.sum() if est.sum() > 0 else est
    implied = np.array(
        [1.0 / race_odds_win[r["horse_num"]] if race_odds_win.get(r["horse_num"], 0.0) >= 1.0 else np.nan for r in rows]
    )
    extra = sum(1.0 / o for h, o in race_odds_win.items() if h not in field and o >= 1.0)
    over = np.nansum(implied) + extra
    if over <= 0.0 or not np.any(np.isfinite(implied)):
        mixed = e_hat.copy()
    else:
        q = implied / over
        mixed = np.where(np.isfinite(q), DUMP_ALPHA * e_hat + (1.0 - DUMP_ALPHA) * q, e_hat)
        mixed = np.minimum(mixed / mixed.sum(), 1.0)
    powered = np.power(mixed, DUMP_GAMMA)
    return np.minimum(powered / powered.sum(), 1.0)


def classify_mismatches(
    races: "OrderedDict[str, list[dict]]", mismatched: list[str], race_odds: dict[str, dict[int, float]]
) -> tuple[list[str], list[str]]:
    """再構成が dump と一致しないレースを (取消馬で説明できる, 説明できない) に分ける。"""
    explained, unexplained = [], []
    for rid in mismatched:
        rows = races[rid]
        ref = np.array([float(r["model_win"]) for r in rows])
        ro = race_odds.get(rid, {})
        has_extra = bool(set(ro) - {r["horse_num"] for r in rows})
        if has_extra and float(np.max(np.abs(reconstruct_with_scratched(rows, ro) - ref))) < 1e-9:
            explained.append(rid)
        else:
            unexplained.append(rid)
    return explained, unexplained


def fetch_race_odds_win(race_ids: list[str]) -> dict[str, dict[int, float]]:
    """`race_odds` の単勝（SELECT のみ）。取消馬の説明に使う。"""
    if not race_ids:
        return {}
    import pgq

    # 値は SQL に補間せず variables で束縛する（pgq の規約）。race_id にカンマは含まれない。
    out: dict[str, dict[int, float]] = {}
    for rid, key, odds in pgq.query(
        "SELECT race_id, combination_key, odds FROM race_odds "
        "WHERE bet_type='win' AND race_id = ANY(string_to_array(:'ids', ','))",
        variables={"ids": ",".join(race_ids)},
    ):
        out.setdefault(rid, {})[int(key)] = float(odds)
    return out


def substitute(rows: list[dict], odds: dict[int, float]) -> list[dict]:
    """dump の行の単勝を時点のオッズに差し替え、本番構成（α=0.2・γ=1.25）の win を再構成した新しい行を返す。"""
    new = [dict(r, win_odds=float(odds[r["horse_num"]])) for r in rows]
    recon = ms.reconstruct_win(new, DUMP_ALPHA, DUMP_GAMMA)
    for r, w in zip(new, recon):
        r["model_win"] = float(w)
    return new


def decide(verdict: str) -> str:
    """事前コミットした判定規則。朝で「採用候補」のときだけ α=0.2 を残す。"""
    return "α=0.2 維持" if verdict == "採用候補" else "α=0 に倒す"


def build_point_tables(
    races: "OrderedDict[str, list[dict]]", snaps: dict[str, dict]
) -> tuple[dict[str, "OrderedDict[str, list[dict]]"], Counter]:
    """3 時点すべてで完全スナップショットがあるレースだけを、時点ごとに差し替えた行で返す（3 時点で同じ集合）。"""
    counts: Counter = Counter()
    tables = {p: OrderedDict() for p in POINTS}
    for rid, rows in races.items():
        entry = snaps.get(rid)
        if entry is None:
            counts["no_snapshot"] += 1
            continue
        pts = select_points(entry, {r["horse_num"] for r in rows})
        if pts is None:
            counts["no_post_time"] += 1
            continue
        if any(pts[p] is None for p in POINTS):
            counts["no_complete_point"] += 1
            continue
        disc = morning_disclosure(entry, {r["horse_num"] for r in rows})
        counts["morning_late"] += int(disc["late"])
        counts["morning_differs_from_first_win"] += int(disc["differs"])
        for p in POINTS:
            tables[p][rid] = substitute(rows, pts[p])
    return tables, counts


def evaluate_point(table: "OrderedDict[str, list[dict]]", n_boot: int, seed: int) -> dict:
    """1 時点の market と blended(α=0.2) を model_eval の土俵で比べる。"""
    elig = me.eligible_races([r for rr in table.values() for r in rr])
    stats = {
        s: [me.per_race_stats(rr, me.system_probs(rr, s)) for rr in elig.values()] for s in ("market", "blended")
    }
    samples = me.bootstrap_samples(stats, n_boot, seed)
    cis = me.ci_from_samples(samples)
    points = {s: me.aggregate(st) for s, st in stats.items()}
    delta = me.delta_ci(stats, "blended", "market", samples=samples)
    days = len({rr[0]["date"] for rr in elig.values()})
    return {
        "races": len(elig),
        "days": days,
        "points": points,
        "ci": {s: {k: list(v) for k, v in c.items()} for s, c in cis.items()},
        "delta": {k: list(v) for k, v in delta.items()},
        "verdict": me.judge(delta),
    }


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dump", help="analyze backtest --dump-features の TSV（本番 5 フラグ）")
    ap.add_argument("--bootstrap", type=int, default=1000)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--json", default=None)
    args = ap.parse_args(argv)
    if args.bootstrap < 1 or args.seed < 0:
        ap.error("--bootstrap は 1 以上、--seed は 0 以上にしてください")
    if not os.path.isfile(args.dump):
        ap.error(f"dump が見つかりません: {args.dump}")
    if args.json and os.path.realpath(args.json) == os.path.realpath(args.dump):
        ap.error(f"--json が dump と同じパスです（dump が上書きされます）: {args.json}")

    sha = me.file_sha256(args.dump)
    rows = pe.load_dump(args.dump)
    usable = me.eligible_races(rows)

    # 忠実性: (α=0.2, γ=1.25) の再構成が dump の model_win と一致するかで、再構成の式を確かめる（#284 と同じ）。
    # 一致しないレースのうち、取消馬（race_odds に残る dump 外の馬）を分母に入れれば一致するものは「説明できる」
    # として比較に含める（各時点の比較は出走馬だけで正規化するので dump の当時の正規化に依存しない）。
    # 説明できない不一致が 1% を超えたら停止する（#218 事前コミットの修正・2026-11-03）。
    snaps = zsp.fetch_snapshots_with_cards()
    in_scope = OrderedDict((rid, rr) for rid, rr in usable.items() if rid in snaps)
    ok, mismatched, worst = ms.fidelity_split(in_scope)
    explained, unexplained = classify_mismatches(in_scope, mismatched, fetch_race_odds_win(mismatched))
    check_fidelity(len(unexplained), len(in_scope))
    usable_races = OrderedDict((rid, rr) for rid, rr in in_scope.items() if rid not in set(unexplained))

    tables, counts = build_point_tables(usable_races, snaps)
    results = {p: evaluate_point(tables[p], args.bootstrap, args.seed) for p in POINTS}
    verdict = results[DECISION_POINT]["verdict"]
    decision = decide(verdict)

    print(f"== #218 朝〜直前のオッズでの α 判定（dump sha256 {sha} / bootstrap {args.bootstrap} 回・seed {args.seed}） ==")
    print(
        f"スナップショットのある dump レース {len(in_scope)}（勝者一意・オッズ/馬番完全）/ 再構成一致 {len(ok)}"
        f"（最大差 {worst:.1e}）・取消馬で説明 {len(explained)}・説明できず除外 {len(unexplained)}"
        f" / 3 時点そろわず除外 {counts['no_complete_point']} / 発走時刻なし {counts['no_post_time']}"
    )
    print(
        f"朝の時点の開示（判定には使わない）: 発走 40 分前以降 {counts['morning_late']}R（本番なら朝の選定が無い）"
        f" / その日の最初の単勝スナップショットと別 {counts['morning_differs_from_first_win']}R（全馬そろい待ち）"
    )
    print("注: blended の place/show 系の指標は dump の値（時点で再構成していない）。判定の主指標は win と ◎ から計算する。")
    for p in POINTS:
        r = results[p]
        print(f"\n[{POINT_LABEL[p]}] {r['races']}R・{r['days']} 日")
        for k in ("win_hit", "place_hit", "show_hit", "top5_win_all3", "win_brier", "win_logloss"):
            pct = not (k.endswith("brier") or k.endswith("logloss"))
            fm = (lambda x: f"{x * 100:5.1f}%") if pct else (lambda x: f"{x:.4f}")
            m, b = r["points"]["market"][k], r["points"]["blended"][k]
            (mlo, mhi), (blo, bhi) = r["ci"]["market"][k], r["ci"]["blended"][k]
            print(f"  {k:14} market {fm(m)} [{fm(mlo)}, {fm(mhi)}] | α=0.2 {fm(b)} [{fm(blo)}, {fm(bhi)}]")
        print("  Δ(α=0.2 − market):")
        for k, (pt, lo, hi) in r["delta"].items():
            pct = not k.endswith("logloss")
            fm = (lambda x: f"{x * 100:+.1f}pt") if pct else (lambda x: f"{x:+.4f}")
            print(f"    {k:14} {fm(pt)} [{fm(lo)}, {fm(hi)}]")
        print(f"  判定: {r['verdict']}" + ("" if p == DECISION_POINT else "（参考）"))
    print(f"\n結論（事前コミット: 朝で「採用候補」なら維持、それ以外は α=0）: 朝 = {verdict} → **{decision}**")

    if args.json:
        payload = {
            "dump": {"path": args.dump, "sha256_12": sha},
            "args": {"bootstrap": args.bootstrap, "seed": args.seed},
            "population": {
                "in_scope": len(in_scope),
                "fidelity_exact": len(ok),
                "fidelity_explained_by_scratched": explained,
                "fidelity_unexplained_excluded": unexplained,
                "fidelity_worst_ok": worst,
                **{k: int(v) for k, v in counts.items()},
            },
            "points": results,
            "note": "blended の place/show/top5_show 系の指標は dump の値（時点で再構成していない）。判定の主指標は win と ◎ から計算する",
            "decision_point": DECISION_POINT,
            "decision": decision,
        }
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump(payload, f, ensure_ascii=False, indent=2)
        print(f"JSON: {args.json}")


if __name__ == "__main__":
    main()
