#!/usr/bin/env python3
"""ズレ増額の符号検証（#706）— 直前オッズ変化の方向 × 実現回収率の実測。

現行ルール（CLAUDE.md「軸ロックとズレ増額」/ ADR 0060）は「軸馬が過小人気にズレたら増額」。
Hanyu et al. (2025, arXiv:2509.14645) は同一最終オッズ条件付きで「直前にオッズが下がった馬」の
実現回収率が高い（15 分窓 −0.105）と逆符号を示す。race_odds_snapshots(win) で実測する。

設計（判定基準は issue #706 コメントで実装前に事前コミット済み）:
  - t_last = 発走前（lag ≥ 0・JST 日付一致）最後の snapshot。t_prev = そこから
    MIN_GAP_MIN 分以上前の最近傍（収集間隔が 1〜16 分と不定のため固定インデックス不可）。
  - シグナル Δ = ln(odds_prev) − ln(odds_last)。**Δ>0 = オッズ低下 = 締まった**。
  - 回収・条件付けはともに**確定単勝オッズ**（t_last オッズで条件付けると t_last→確定間の
    ドリフトが文献方向を打ち消すバイアスになる）。確定オッズは results を優先し、不足は
    netkeiba 結果ページの単勝オッズ列（`td.Odds.Txt_R`）。単勝 100 円の回収 = 勝ちなら
    確定オッズ×100・負けなら 0。
  - 層別: ln(確定オッズ) 五分位 × Δ>0 / Δ=0 / Δ<0 の 3 群平均回収 + 層サイズ加重差。
  - 補助推定: won ~ a + b·ln(確定オッズ) + δ·Δ の binary ロジスティック（ニュートン法）。
    レース単位 bootstrap（n=2000・seed=42・percentile 95% CI・非収束 skip 件数報告）。
  - 判定（judge）: δ̂ CI が 0 を跨がず正 かつ 層別加重差（全体・低オッズ層とも）が同方向
    → 改訂案提示（採否は PO）。それ以外は現行維持（inconclusive は現行維持側）。

依存: psql (PADDOCK_DB_URL)・標準ライブラリ・同ディレクトリ nk.py / late_money_probe.py /
odds_guard.py。netkeiba キャッシュは late_money_probe の .cache_nk_results を共有。
"""

from __future__ import annotations

import argparse
import json
import math
import os
import random
import re
import subprocess
import sys
import time
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import late_money_probe as lmp  # noqa: E402  parse_ts / slug_to_netkeiba_id / CACHE を共有
import nk  # noqa: E402
import odds_guard  # noqa: E402

DB = os.environ.get("PADDOCK_DB_URL", "postgres://paddock:paddock@127.0.0.1:5432/paddock")
CACHE = lmp.CACHE  # .cache_nk_results（着順キャッシュと同居・二重取得を避ける）

JST_OFFSET_SEC = 9 * 3600
MIN_GAP_MIN = 10.0  # t_prev は t_last の 10 分以上前（実質 15 分粒度の下限側）
N_QUANT = 5
N_BOOT = 2000
SEED = 42
LOW_STRATA = (0, 1)  # 軸馬相当帯 = ln(確定オッズ) の第 1〜2 五分位（低オッズ側）


# ---------- 純粋関数（テスト対象） ----------

def parse_utc_ts(s: str) -> int:
    """fetched_at（rfc3339）→ UTC epoch 秒。オフセットが UTC 以外なら停止する。

    lmp.parse_ts は「UTC 保存前提・相対差専用」でオフセットを読み捨てる。本スクリプトは
    絶対時刻（JST 日付・発走 lag）に使うので、+09:00 等が来たら黙って 9 時間ずれる前に止める。
    """
    if not re.search(r"(\+00(:?00)?|Z)$", s):
        raise ValueError(f"UTC 以外のオフセットの fetched_at: {s!r}")
    return lmp.parse_ts(s)


def hhmm_to_min(s: str | None) -> int | None:
    """'15:10' -> 910。post_time は JST HH:MM（NULL 許容）。"""
    if not s:
        return None
    m = re.fullmatch(r"(\d{1,2}):(\d{2})", s)
    if not m:
        return None
    return int(m.group(1)) * 60 + int(m.group(2))


def jst_date_min(epoch_utc: int) -> tuple[str, float]:
    """UTC epoch 秒 -> (JST 日付 'YYYY-MM-DD', JST の 0 時からの分)。"""
    t = time.gmtime(epoch_utc + JST_OFFSET_SEC)
    date = f"{t.tm_year:04d}-{t.tm_mon:02d}-{t.tm_mday:02d}"
    return date, t.tm_hour * 60 + t.tm_min + t.tm_sec / 60.0


def select_points(
    ts_sorted: list[int], race_date: str, post_min: int
) -> tuple[int, int] | tuple[None, str]:
    """レースの snapshot 時刻集合から (t_prev, t_last) を選ぶ。

    有効 = JST 日付が race_date と一致 かつ 発走前（lag ≥ 0）。翌日再スクレイプ混入
    （#568 既知経路）と発走後取得をここで落とす。t_last = 有効の最後、t_prev = t_last の
    MIN_GAP_MIN 分以上前で最も近いもの。選べなければ (None, 理由)。
    """
    valid = []
    for ts in ts_sorted:
        d, mn = jst_date_min(ts)
        if d == race_date and mn <= post_min:
            valid.append(ts)
    if not valid:
        return None, "no_prerace_snapshot"
    t_last = valid[-1]
    prevs = [t for t in valid if (t_last - t) >= MIN_GAP_MIN * 60]
    if not prevs:
        return None, "no_prev_point"
    return prevs[-1], t_last


def parse_result_odds(html: str) -> dict[int, float]:
    """netkeiba 結果ページから {馬番: 確定単勝オッズ} を抽出する純粋関数。

    行構造は nk.fetch_result と同じ HorseList 行。オッズは `td class="Odds Txt_R"` 内の
    span（1 着馬のみ class="Odds_Ninki" 付き）。取消/除外馬はセル自体が無いのでスキップ。
    """
    out: dict[int, float] = {}
    for r in re.findall(r'class="HorseList">(.*?)</tr>', html, re.S):
        cell = re.search(r'<td[^>]*class="Odds Txt_R"[^>]*>(.*?)</td>', r, re.S)
        if not cell:
            continue
        o = re.search(r"<span[^>]*>\s*([\d.]+)\s*</span>", cell.group(1))
        if not o:
            continue
        mnum = re.search(r'class="Num[^"]*Txt_C[^"]*">\s*<div[^>]*>\s*(\d+)\s*</div>', r)
        if not mnum:
            continue
        try:
            out[int(mnum.group(1))] = float(o.group(1))
        except ValueError:
            continue
    return out


def quantile_bounds(values: list[float], k: int = N_QUANT) -> list[float]:
    """五分位境界（内部境界 k-1 個）。assign_bin とペアで使う。"""
    xs = sorted(values)
    n = len(xs)
    return [xs[min(n - 1, (n * i) // k)] for i in range(1, k)]


def assign_bin(x: float, bounds: list[float]) -> int:
    for i, b in enumerate(bounds):
        if x < b:
            return i
    return len(bounds)


def group_of(delta: float) -> str:
    """Δ>0 = オッズ低下（締まった）/ Δ=0 / Δ<0 = オッズ上昇（緩んだ）。"""
    if delta > 0:
        return "+"
    if delta < 0:
        return "-"
    return "0"


def strata_summary(records: list[dict], bounds: list[float]) -> dict:
    """五分位 × 3 群の (n, 平均回収) と、層サイズ加重の Δ>0 vs Δ≤0 差を返す。

    加重差 = Σ_bin w_bin·(mean_ret(Δ>0) − mean_ret(Δ≤0))、w_bin = 両群が居る bin の頭数比。
    低オッズ層（LOW_STRATA）だけの加重差も返す（軸馬相当帯の方向一致条件）。
    """
    cells: dict[tuple[int, str], list[float]] = defaultdict(list)
    for r in records:
        b = assign_bin(math.log(r["final_odds"]), bounds)
        cells[(b, group_of(r["delta"]))].append(r["ret"])

    def wdiff(bins: range | tuple) -> tuple[float, int]:
        num, den = 0.0, 0
        for b in bins:
            up = cells.get((b, "+"), [])
            dn = cells.get((b, "0"), []) + cells.get((b, "-"), [])
            if not up or not dn:
                continue
            n = len(up) + len(dn)
            num += n * (sum(up) / len(up) - sum(dn) / len(dn))
            den += n
        return (num / den if den else float("nan")), den

    overall, n_overall = wdiff(range(N_QUANT))
    low, n_low = wdiff(LOW_STRATA)
    return {
        "cells": {k: (len(v), sum(v) / len(v)) for k, v in cells.items()},
        "weighted_diff": overall,
        "weighted_diff_n": n_overall,
        "low_weighted_diff": low,
        "low_weighted_diff_n": n_low,
    }


def fit_logistic(records: list[dict], max_iter: int = 50, tol: float = 1e-10):
    """won ~ a + b·ln(final_odds) + δ·delta の 3 係数ニュートン法。

    返り値 (a, b, delta_hat, converged)。分離・特異で収束しなければ converged=False
    （bootstrap 側で skip して件数を数える）。
    """
    X = [(1.0, math.log(r["final_odds"]), r["delta"]) for r in records]
    y = [r["won"] for r in records]
    w = [0.0, 0.0, 0.0]
    for _ in range(max_iter):
        g = [0.0] * 3
        H = [[0.0] * 3 for _ in range(3)]
        for xi, yi in zip(X, y):
            z = sum(wj * xj for wj, xj in zip(w, xi))
            z = max(-30.0, min(30.0, z))
            p = 1.0 / (1.0 + math.exp(-z))
            for j in range(3):
                g[j] += (yi - p) * xi[j]
                for kk in range(3):
                    H[j][kk] += p * (1.0 - p) * xi[j] * xi[kk]
        step = _solve3(H, g)
        if step is None:
            return w[0], w[1], w[2], False
        w = [wj + sj for wj, sj in zip(w, step)]
        if max(abs(s) for s in step) < tol:
            return w[0], w[1], w[2], True
    return w[0], w[1], w[2], False


def _solve3(A, b):
    """3x3 のガウス消去（部分ピボット）。特異なら None。"""
    M = [row[:] + [bi] for row, bi in zip(A, b)]
    for c in range(3):
        piv = max(range(c, 3), key=lambda r: abs(M[r][c]))
        if abs(M[piv][c]) < 1e-12:
            return None
        M[c], M[piv] = M[piv], M[c]
        for r in range(3):
            if r != c:
                f = M[r][c] / M[c][c]
                M[r] = [M[r][j] - f * M[c][j] for j in range(4)]
    return [M[r][3] / M[r][r] for r in range(3)]


def bootstrap_delta_ci(
    by_race: dict[str, list[dict]], n_boot: int = N_BOOT, seed: int = SEED
) -> tuple[float, float, int]:
    """レース単位リサンプルの δ̂ percentile 95% CI。非収束の再標本は skip して数える。"""
    rng = random.Random(seed)
    rids = list(by_race.keys())
    deltas = []
    n_skip = 0
    for _ in range(n_boot):
        sample: list[dict] = []
        for _ in rids:
            sample.extend(by_race[rng.choice(rids)])
        _, _, d, ok = fit_logistic(sample)
        if ok:
            deltas.append(d)
        else:
            n_skip += 1
    if not deltas:
        raise SystemExit(f"bootstrap の全 {n_boot} 再標本が非収束。データを確認。")
    deltas.sort()
    n = len(deltas)
    lo = deltas[int(math.floor(0.025 * n))]
    hi = deltas[max(0, int(math.ceil(0.975 * n)) - 1)]
    return lo, hi, n_skip


def judge(ci_lo: float, ci_hi: float, weighted_diff: float, low_weighted_diff: float) -> str:
    """事前コミット判定（issue #706 コメントで実装前固定）。

    改訂案提示 = δ̂ 95% CI が 0 を跨がず正（Δ>0 = オッズ低下馬が有利 = 文献方向）かつ
    層別加重差が全体・低オッズ層（軸馬相当帯）とも正。それ以外（CI が 0 跨ぎ / 負 /
    層別と食い違い / NaN）は現行維持。
    """
    literature_direction = ci_lo > 0 and ci_hi > 0
    strata_agree = (
        not math.isnan(weighted_diff)
        and not math.isnan(low_weighted_diff)
        and weighted_diff > 0
        and low_weighted_diff > 0
    )
    if literature_direction and strata_agree:
        return "改訂案提示"
    return "現行維持"


# ---------- データ取得 ----------

def _psql(sql: str) -> list[list[str]]:
    out = subprocess.run(
        ["psql", DB, "-At", "-F", "\t", "-c", sql],
        capture_output=True, text=True, check=True,
        env={**os.environ, "PGCONNECT_TIMEOUT": "5"},
    ).stdout
    return [line.split("\t") for line in out.splitlines()]


def fetch_snapshots_with_cards():
    """(race_id, 馬番, odds, epoch, race_date, post_min) の行と、card 欠落レース数を返す。"""
    rows = _psql(
        "SELECT s.race_id, s.combination_key, s.odds, s.fetched_at, c.date, c.post_time "
        "FROM race_odds_snapshots s LEFT JOIN race_cards c ON c.race_id = s.race_id "
        "WHERE s.bet_type='win' ORDER BY s.race_id, s.fetched_at"
    )
    by_race: dict[str, dict] = {}
    for rid, num, odds, fetched, date, post in rows:
        e = by_race.setdefault(rid, {"date": date, "post_min": hhmm_to_min(post),
                                     "snaps": defaultdict(dict)})
        o = float(odds)
        if odds_guard.is_payout_odds("win", o):
            e["snaps"][parse_utc_ts(fetched)][int(num)] = o
    return by_race


def fetch_results_db() -> dict[str, dict[int, tuple[int | None, float | None]]]:
    """results から {race_id: {馬番: (着順, 確定単勝オッズ)}}（NULL は None）。"""
    rows = _psql(
        "SELECT race_id, horse_num, COALESCE(finishing_position::text,''), "
        "COALESCE(odds::text,'') FROM results"
    )
    out: dict[str, dict[int, tuple[int | None, float | None]]] = defaultdict(dict)
    for rid, num, fin, odds in rows:
        out[rid][int(num)] = (int(fin) if fin else None, float(odds) if odds else None)
    return out


def fetch_final_odds_nk(nk_id: str) -> dict[int, float]:
    """netkeiba 結果ページの確定単勝オッズ（キャッシュ付き・late_money_probe と同じ礼儀）。"""
    path = os.path.join(CACHE, f"{nk_id}_odds.json")
    if os.path.exists(path):
        return {int(k): v for k, v in json.load(open(path, encoding="utf-8")).items()}
    html = nk.decode(nk.curl(f"https://race.netkeiba.com/race/result.html?race_id={nk_id}"))
    odds = parse_result_odds(html)
    if odds:  # 空（未生成・構造変化）はキャッシュしない（lmp.fetch_finish と同方針）
        with open(path, "w", encoding="utf-8") as f:
            json.dump(odds, f)
    time.sleep(1.5)
    return odds


def fetch_win_payout_nk(nk_id: str) -> dict[int, int]:
    """netkeiba 払戻ブロックの単勝 {馬番: 100 円あたり払戻}（整合検査用・キャッシュ付き）。"""
    path = os.path.join(CACHE, f"{nk_id}_win_payout.json")
    if os.path.exists(path):
        return {int(k): v for k, v in json.load(open(path, encoding="utf-8")).items()}
    payouts = nk.fetch_payouts(nk_id).get("win", {})
    out = {int(k): v for k, v in payouts.items()}
    if out:
        with open(path, "w", encoding="utf-8") as f:
            json.dump(out, f)
    time.sleep(1.5)
    return out


# ---------- main ----------

def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bootstrap", type=int, default=N_BOOT)
    ap.add_argument("--seed", type=int, default=SEED)
    ap.add_argument("--no-crosscheck", action="store_true",
                    help="results×netkeiba 払戻の整合検査を省略（オフライン再集計用）")
    args = ap.parse_args()
    if args.bootstrap < 1:
        ap.error("--bootstrap は 1 以上")

    by_race = fetch_snapshots_with_cards()
    results_db = fetch_results_db()
    print(f"母集団: snapshot(win) レース {len(by_race)}R")

    excl = defaultdict(int)
    windows: list[float] = []
    records: list[dict] = []
    rec_by_race: dict[str, list[dict]] = defaultdict(list)
    head_drop: dict[str, int] = defaultdict(int)
    crosscheck_n: dict[str, int] = defaultdict(int)
    crosscheck_bad: dict[str, int] = defaultdict(int)
    crosscheck_miss: dict[str, int] = defaultdict(int)
    crosscheck_fail = 0

    for rid, e in sorted(by_race.items()):
        if not e["date"] or e["post_min"] is None:
            excl["card欠落/発走時刻なし"] += 1
            continue
        ts_sorted = sorted(e["snaps"])
        sel = select_points(ts_sorted, e["date"], e["post_min"])
        if sel[0] is None:
            excl[sel[1]] += 1
            continue
        t_prev, t_last = sel

        # 確定オッズ・着順: results 優先、無ければ netkeiba（着順キャッシュは lmp と共有）。
        nk_id = lmp.slug_to_netkeiba_id(rid)
        finals: dict[int, float] = {}
        fins: dict[int, int] = {}
        src = "results"
        if rid in results_db:
            for num, (fin, odds) in results_db[rid].items():
                if fin is not None:
                    fins[num] = fin
                if odds is not None:
                    finals[num] = odds
        if (not finals or not fins) and nk_id:
            try:
                if not fins:
                    fins = lmp.fetch_finish(nk_id)
                if not finals:
                    finals = fetch_final_odds_nk(nk_id)
                    src = "netkeiba"
            except Exception as ex:
                print(f"  ! {rid} ({nk_id}) 取得失敗: {ex}", file=sys.stderr)
        # 確定オッズも snapshot と同じ odds_guard を通す（≤0 は log で落ち、<1 は回収を汚す）。
        n_bad_odds = sum(1 for v in finals.values() if not odds_guard.is_payout_odds("win", v))
        if n_bad_odds:
            head_drop["確定オッズ値域外"] += n_bad_odds
            finals = {k: v for k, v in finals.items() if odds_guard.is_payout_odds("win", v)}
        if not finals or not fins:
            excl["確定オッズ/着順なし"] += 1
            continue

        # 1 着同着は払戻が按分され「確定オッズ×100」にならないので除外（#703 の winner_races と同じ）。
        winners = [n for n, f in fins.items() if f == 1]
        if len(winners) > 1:
            excl["1着同着"] += 1
            continue
        if not winners or any(w not in finals for w in winners):
            excl["勝ち馬の確定オッズ/着順なし"] += 1
            continue

        # 整合検査: 確定オッズ×100 == netkeiba 単勝払戻（results 経路・netkeiba パーサ経路の両方）。
        if nk_id and not args.no_crosscheck:
            try:
                pay = fetch_win_payout_nk(nk_id)
                for num, p in pay.items():
                    if num in finals:
                        crosscheck_n[src] += 1
                        if abs(finals[num] * 100 - p) > 0.5:
                            crosscheck_bad[src] += 1
                            print(f"  ! 整合検査 NG {rid}（{src}）馬{num}: "
                                  f"{finals[num]}×100 vs 払戻 {p}", file=sys.stderr)
                    else:
                        crosscheck_miss[src] += 1  # 払戻はあるのに確定オッズ側に馬番が無い
            except Exception as ex:
                crosscheck_fail += 1
                print(f"  ! {rid} 払戻取得失敗（整合検査のみ skip）: {ex}", file=sys.stderr)

        # 勝ち馬の確定オッズが取れないレースは全馬 ret=0 で残り回収を押し下げるので除外。
        prev_odds = e["snaps"][t_prev]
        last_odds = e["snaps"][t_last]
        race_recs = []
        for num in sorted(set(prev_odds) & set(last_odds)):
            if num not in finals or num not in fins:
                head_drop["結果側に馬番なし（取消等）"] += 1
                continue
            delta = math.log(prev_odds[num]) - math.log(last_odds[num])
            won = 1 if fins[num] == 1 else 0
            race_recs.append({
                "race": rid, "num": num, "delta": delta,
                "final_odds": finals[num], "won": won,
                "ret": finals[num] * 100.0 if won else 0.0,
            })
        if not race_recs:
            excl["snapshot×結果の馬番不一致"] += 1
            continue
        records.extend(race_recs)
        rec_by_race[rid] = race_recs
        windows.append((t_last - t_prev) / 60.0)  # 実効レースのみ（除外レースを混ぜない）

    n_eff = len(rec_by_race)
    print(f"実効 N: {n_eff}R / {len(records)} 頭")
    print("除外内訳（レース）: "
          + (" / ".join(f"{k}={v}" for k, v in sorted(excl.items())) or "なし"))
    print("除外内訳（頭）: "
          + (" / ".join(f"{k}={v}" for k, v in sorted(head_drop.items())) or "なし"))
    if windows:
        ws = sorted(windows)
        print(f"実効窓 t_last−t_prev [分]: 最小 {ws[0]:.1f} / 中央 {ws[len(ws)//2]:.1f}"
              f" / 最大 {ws[-1]:.1f}")
    if args.no_crosscheck:
        print("整合検査: 未実施（--no-crosscheck）")
    else:
        parts = [f"{s} 経路 {crosscheck_n[s]} 点照合・不一致 {crosscheck_bad[s]}"
                 f"・確定オッズ側に馬番なし {crosscheck_miss[s]}"
                 for s in ("results", "netkeiba")]
        print("整合検査（確定オッズ×100 vs netkeiba 単勝払戻）: " + " / ".join(parts)
              + f" / 払戻取得失敗 {crosscheck_fail}R")
        if sum(crosscheck_n.values()) == 0:
            print("  ⚠ 照合 0 点（整合検査が実質未実施）", file=sys.stderr)
    if len(records) < 100:
        raise SystemExit("サンプル過少（<100 頭）。中断。")

    n_zero = sum(1 for r in records if r["delta"] == 0.0)
    print(f"Δ=0 の割合: {n_zero}/{len(records)} ({n_zero / len(records):.1%})"
          "（オッズ刻みが粗い帯では Δ=0 が Δ≤0 群を薄める点に注意）")

    bounds = quantile_bounds([math.log(r["final_odds"]) for r in records])
    s = strata_summary(records, bounds)
    print("\n-- ln(確定オッズ) 五分位 × Δ 3 群の平均回収（円/100 円） --")
    print(f"  {'五分位':>6} {'群':>3} {'n':>6} {'平均回収':>9}")
    for b in range(N_QUANT):
        for g in ("+", "0", "-"):
            if (b, g) in s["cells"]:
                n, m = s["cells"][(b, g)]
                tag = "Δ>0=低下" if g == "+" else ("Δ<0=上昇" if g == "-" else "Δ=0")
                print(f"  Q{b + 1:>5} {g:>3} {n:>6} {m:>9.1f}  {tag}")
    print(f"層サイズ加重差（Δ>0 − Δ≤0）: 全体 {s['weighted_diff']:+.1f} 円"
          f"（n={s['weighted_diff_n']}）/ 低オッズ層 Q1-2 {s['low_weighted_diff']:+.1f} 円"
          f"（n={s['low_weighted_diff_n']}）")

    a, b_coef, d_hat, ok = fit_logistic(records)
    if not ok:
        raise SystemExit("点推定のロジスティックが非収束。データを確認。")
    lo, hi, n_skip = bootstrap_delta_ci(rec_by_race, n_boot=args.bootstrap, seed=args.seed)
    print(f"\n-- 補助推定: won ~ a + b·ln(確定オッズ) + δ·Δ --")
    print(f"  a={a:+.3f} b={b_coef:+.3f} δ̂={d_hat:+.4f}")
    print(f"  δ̂ 95% CI [{lo:+.4f}, {hi:+.4f}]（レース単位 bootstrap n={args.bootstrap}"
          f"・seed={args.seed}・percentile・非収束 skip {n_skip}）")

    verdict = judge(lo, hi, s["weighted_diff"], s["low_weighted_diff"])
    print(f"\n事前コミット判定: {verdict}")
    print("限定条件: 実効 N は Hanyu et al. の 63,000R に対し大きく劣る（有意でない ≠ 効果なし）"
          "/ 収集は実質 15 分粒度（5 分効果は原理的に測れない）"
          "/ 単勝回収は増額対象券種（ワイド/馬連/3連複）の proxy")


if __name__ == "__main__":
    main()
