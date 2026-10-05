"""買い目を固める時点の比較（#724）: 朝の選定 vs 発走 40 分前（T-40）の固定.

入力は `paddock-analyze reconstruct-slips --date D` の出力（1 レース 1 行の JSON・同じ Rust の
`compose_portfolio` で組んだ両時点の伝票。買い方を Python で組み直さない＝ADR 0064）。
払戻と非出走は netkeiba の結果ページのキャッシュを直接読む。ネットワークには出ず、キャッシュも消さない
（キャッシュに無いレースは「払戻なし」、壊れたキャッシュは「truncated_cache」で外して件数を出す）。

判定規則は実装前に issue #724 で固定した（「実装計画の改訂」）:
- 主判定は 09-27 を除いた日（09-27 は仮説が生まれた日）。全日でも同じ結論かを併記する。
- 同じレースの「朝 − T-40」のプール ROI の差について、レース単位ブートストラップ（2000 回・seed 571）の
  95% CI が 0 を跨がず（正の側）、かつ差が 0 でない日のうち 7 割以上で向きが一致したときだけ前倒しを提案。
- 2 次の指標（軸の的中率・複勝率・食い違い率・上位 3 レースを除いた値・開催日を 1 日ずつ抜いた値・
  時間帯の層別）は記述用で、判定には使わない。

使い方:
  python3 fix_timing_probe.py <reconstruct-slips の jsonl ...>             # 精算と判定
  python3 fix_timing_probe.py --check-recorded [--axis-prob TSV] <jsonl ...>  # 記録された T-40 の固定との一致
（結果ページのキャッシュの場所は環境変数 NK_RESULT_CACHE_DIR で変えられる。既定は nk.RESULT_CACHE_DIR。
 --axis-prob は live_ev_snapshots の初回スイープの「race_id<TAB>軸<TAB>軸の勝率(%)」。取り方は一次資料 §7）
"""

import json
import math
import os
import random
import re
import sys
from collections import defaultdict
from datetime import datetime, timedelta, timezone

import gate_calibration as G
import late_money_probe as LM
import nk

JST = timezone(timedelta(hours=9))
HOLDOUT_DAY = "2026-09-27"  # 仮説が生まれた日（主判定から外す）
MIN_GAP_MIN = 60
MAX_T40_LAG_SEC = 300  # predict-watch のスイープ間隔（5 分）。これを超えたら次のスイープの保存を拾っている
DAY_AGREE = 0.7
Z_ALPHA, Z_POWER = 1.959964, 0.841621  # 両側 5%・検出力 80%


def non_runners(html):
    """結果ページで着順のセルが「取消」「除外」の馬番（＝返還の対象）。競走中止は走っているので含めない。"""
    out = set()
    for r in re.findall(r'class="HorseList">(.*?)</tr>', html, re.S):
        rank = re.search(r'class="Rank">\s*([^<\s]+)\s*<', r)
        num = re.search(r'class="Num[^"]*Txt_C[^"]*">\s*<div[^>]*>\s*(\d+)\s*</div>', r)
        if rank and num and rank.group(1) in ("取消", "除外"):
            out.add(int(num.group(1)))
    return out


def finish_ranks(rows):
    """馬番 → 着順。結果ページの後ろにある別の表の行（名前が空・着順なし）は捨て、最初に出た行を採る。

    古いテンプレートの結果ページでは、結果表の後ろの行が 1 着馬の馬番で読めてしまい、後勝ちの辞書だと
    1 着馬の着順が None に上書きされる（#724 の計測で軸の勝率が 4% と出た原因）。
    """
    out = {}
    for x in rows:
        if x.get("name"):
            out.setdefault(x["horse_num"], x["rank"])
    return out


def settle_with_refund(legs, payouts, scratched):
    """返還つきの精算 → (stake, ret, 返還した脚の数)。非出走の馬を含む脚は賭金にも払戻にも入れない。"""
    kept, refunded = [], 0
    for leg in legs:
        if float(leg.get("amount") or 0) <= 0:
            continue
        if scratched & set(leg.get("combo") or []):
            refunded += 1
            continue
        kept.append(leg)
    stake, ret, _, _ = G.settle(kept, payouts)
    return stake, ret, refunded


def _ts(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00"))


def exclusion_reason(r, min_gap_min=MIN_GAP_MIN):
    """計測から外す理由（外さないなら None）。"""
    if r.get("skip") or not r.get("t40") or not r.get("morning_fixed"):
        return "skip"
    if not r["t40"].get("has_ev"):
        return "no_ev"  # live は EV が出ないレースをスキップする
    gap = (_ts(r["captured_at"]) - _ts(r["morning_at"])).total_seconds() / 60
    if gap < min_gap_min:
        return "gap"
    if t40_lag_sec(r) > MAX_T40_LAG_SEC:
        return "t40_late"  # 初回スイープの保存が無く、後のスイープのスナップショットを拾った
    return None


def t40_lag_sec(r):
    """T-40 のスナップショットの取得時刻 − 初回スイープの開始時刻（秒）。"""
    return (_ts(r["t40_at"]) - _ts(r["captured_at"])).total_seconds()


def read_cached_result(cache_dir, rid):
    """結果ページのキャッシュを読むだけ → (html, 読めない理由)。取り直しも削除もしない。

    nk.ResultPages は壊れたキャッシュを消して取り直すので、計測（キャッシュだけ読む）には使わない。
    """
    # race_id はファイル名に入るので 12 桁の数字に限る（nk.ResultPages と同じ検査）
    if not rid or not re.fullmatch(r"[0-9]{12}", rid):
        return None, "no_payout"
    try:
        with open(os.path.join(cache_dir, f"{rid}.html"), "rb") as f:
            raw = f.read()
    except FileNotFoundError:
        return None, "no_payout"
    if nk.is_truncated(raw):
        return None, "truncated_cache"
    return nk.decode(raw), None


def pooled_diff(races):
    """朝のプール ROI − T-40 のプール ROI（比率。賭金ゼロの側があれば None）。"""
    sm = sum(r["m_stake"] for r in races)
    st = sum(r["t_stake"] for r in races)
    if sm <= 0 or st <= 0:
        return None
    return sum(r["m_ret"] for r in races) / sm - sum(r["t_ret"] for r in races) / st


def bootstrap_diff_ci(races, iters=2000, seed=571):
    """レース単位で対のまま復元抽出した pooled_diff の 95% CI。"""
    if len(races) < 2:
        return None
    rnd = random.Random(seed)
    n = len(races)
    vals = []
    for _ in range(iters):
        v = pooled_diff([races[rnd.randrange(n)] for _ in range(n)])
        if v is not None:
            vals.append(v)
    if not vals:
        return None
    vals.sort()
    return vals[int(0.025 * len(vals))], vals[min(len(vals) - 1, int(0.975 * len(vals)))]


def judge(ci, day_diffs):
    """事前に固定した判定（#724）。"""
    nonzero = [d for d in day_diffs if d is not None and d != 0]
    if ci is None or not nonzero:
        return "T-40 を維持"
    agree = sum(1 for d in nonzero if d > 0) / len(nonzero)
    if ci[0] > 0 and agree >= DAY_AGREE:
        return "前倒しを提案"
    return "T-40 を維持"


def mde(sd, n):
    """朝と T-40 を独立とみなした差の最小検出差（両側 5%・検出力 80%・保守側）。"""
    return (Z_ALPHA + Z_POWER) * math.sqrt(2) * sd / math.sqrt(n)


# ---------- 実データ ----------


def _band(slip):
    return sorted({h for leg in slip["legs"] if leg["method"] == "box" for h in leg["combo"]})


def _partners_differ(a, b):
    """相手の食い違い。軸の入れ替わりは相手として二重に数えない（mismatch と同じ基準）。"""
    return set(a["partners"]) - {b["axis"]} != set(b["partners"]) - {a["axis"]}


def build_races(records, read_html):
    """jsonl の行 → 精算済みのレース（外したものは理由ごとに数える）。read_html(netkeiba id) → (html, 理由)。"""
    races, dropped = [], defaultdict(int)
    for r in records:
        why = exclusion_reason(r)
        if why:
            dropped[why] += 1
            continue
        nid = LM.slug_to_netkeiba_id(r["race_id"])
        html, why = read_html(nid)
        payouts = nk.parse_payouts(html, nid, warn=False) if html else {}
        if not payouts.get("win"):
            dropped[why or "no_payout"] += 1
            continue
        scratched = non_runners(html)
        ranks = finish_ranks(nk.parse_result(html, nid, warn=False))
        ms, mr, mref = settle_with_refund(r["morning_fixed"]["legs"], payouts, scratched)
        ts_, tr, tref = settle_with_refund(r["t40"]["legs"], payouts, scratched)
        morning_jst = _ts(r["morning_at"]).astimezone(JST)
        races.append({
            "race_id": r["race_id"],
            "day": _ts(r["captured_at"]).astimezone(JST).strftime("%Y-%m-%d"),
            "m_stake": ms, "m_ret": mr, "t_stake": ts_, "t_ret": tr,
            "refunded": mref + tref,
            "m_axis": r["morning_fixed"]["axis"], "t_axis": r["t40"]["axis"],
            "m_axis_rank": ranks.get(r["morning_fixed"]["axis"]),
            "t_axis_rank": ranks.get(r["t40"]["axis"]),
            "axis_diff": r["morning_fixed"]["axis"] != r["t40"]["axis"],
            "partner_diff": _partners_differ(r["morning_fixed"], r["t40"]),
            "band_diff": _band(r["morning_fixed"]) != _band(r["t40"]),
            "morning_hour": morning_jst.hour,
            "gap_min": (_ts(r["captured_at"]) - _ts(r["morning_at"])).total_seconds() / 60,
        })
    return races, dict(dropped)


def _roi(races, side):
    s = sum(r[f"{side}_stake"] for r in races)
    return sum(r[f"{side}_ret"] for r in races) / s if s else float("nan")


def _day_diffs(races):
    by = defaultdict(list)
    for r in races:
        by[r["day"]].append(r)
    return {d: pooled_diff(rs) for d, rs in sorted(by.items())}


def _rate(races, key, pred):
    xs = [pred(r[key]) for r in races if r[key] is not None]
    return sum(xs) / len(xs) if xs else float("nan")


def _f(v):
    return "—" if v is None else f"{v:+.3f}"


def report(races, label):
    n = len(races)
    print(f"\n=== {label}: {n}R / {len({r['day'] for r in races})} 日 ===")
    if n < 2:
        print("  レースが足りない")
        return
    diff = pooled_diff(races)
    ci = bootstrap_diff_ci(races)
    per_race_t = [r["t_ret"] / r["t_stake"] for r in races if r["t_stake"] > 0]
    if diff is None or ci is None or len(per_race_t) < 2:
        print("  賭金のあるレースが足りない")
        return
    days = _day_diffs(races)
    mu = sum(per_race_t) / len(per_race_t)
    sd = math.sqrt(sum((x - mu) ** 2 for x in per_race_t) / (len(per_race_t) - 1))
    print(f"  ROI 朝 {_roi(races, 'm'):.3f} / T-40 {_roi(races, 't'):.3f}")
    print(f"  差（朝 − T-40）{diff:+.3f}  95% CI [{ci[0]:+.3f}, {ci[1]:+.3f}]")
    # 判定と同じ標本から出す。独立とみなす値は保守側、対の値は CI の半幅から（SE ≈ 半幅 / 1.96）
    print(f"  MDE: 独立とみなす（T-40 の 1 レース ROI の SD {sd:.3f}）±{mde(sd, n):.3f}"
          f" / 対の差（CI の半幅から）±{(Z_ALPHA + Z_POWER) * (ci[1] - ci[0]) / 2 / Z_ALPHA:.3f}")
    nz = [d for d in days.values() if d]
    print(f"  日別の差: " + ", ".join(f"{d[5:]} {_f(v)}" for d, v in days.items()))
    print(f"  差が 0 でない日のうち朝が上: {sum(1 for d in nz if d > 0)}/{len(nz)}")
    print(f"  判定: {judge(ci, list(days.values()))}")
    # 以下は記述用（判定に使わない）
    print("  -- 記述用 --")
    print(f"  食い違い: 軸 {sum(r['axis_diff'] for r in races)}/{n}・相手 {sum(r['partner_diff'] for r in races)}/{n}"
          f"・混戦 {sum(r['band_diff'] for r in races)}/{n}・返還の脚 {sum(r['refunded'] for r in races)}")
    for side, name in (("m", "朝"), ("t", "T-40")):
        print(f"  軸 {name}: 勝率 {_rate(races, side + '_axis_rank', lambda k: k == 1):.3f}"
              f" / 複勝率 {_rate(races, side + '_axis_rank', lambda k: k <= 3):.3f}")
    div = [r for r in races if r["axis_diff"] or r["partner_diff"] or r["band_diff"]]
    if div:
        print(f"  食い違ったレースだけ: {len(div)}R 朝 ¥{sum(r['m_ret'] for r in div):,.0f} / "
              f"T-40 ¥{sum(r['t_ret'] for r in div):,.0f}（賭金 朝 ¥{sum(r['m_stake'] for r in div):,.0f}・"
              f"T-40 ¥{sum(r['t_stake'] for r in div):,.0f}）")
    top = sorted(races, key=lambda r: -max(r["m_ret"], r["t_ret"]))[3:]
    print(f"  払戻の大きい上位 3 レースを除く差: {_f(pooled_diff(top))}")
    loo = {d: pooled_diff([r for r in races if r["day"] != d]) for d in days}
    print("  1 日抜き: " + ", ".join(f"-{d[5:]} {_f(v)}" for d, v in loo.items()))
    for name, key in (("朝の時刻", lambda r: "〜10時" if r["morning_hour"] < 10 else ("10〜12時" if r["morning_hour"] < 12 else "12時〜")),
                      ("朝→T-40", lambda r: "60〜120分" if r["gap_min"] < 120 else ("120〜240分" if r["gap_min"] < 240 else "240分〜"))):
        by = defaultdict(list)
        for r in races:
            by[key(r)].append(r)
        print(f"  層別（{name}）: " + ", ".join(f"{k} {len(v)}R {_f(pooled_diff(v))}" for k, v in sorted(by.items())))


# ---------- 再構成の検査（記録された T-40 の固定との一致・G2） ----------


def load_axis_prob(path):
    """「race_id<TAB>軸<TAB>軸の勝率(%)」→ {race_id: (軸, 勝率%)}。"""
    out = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            p = line.rstrip("\n").split("\t")
            if len(p) == 3:
                out[p[0]] = (int(p[1]), float(p[2]))
    return out


def mismatch(r):
    """記録の固定と再構成した T-40 の食い違い → [(種類, 記録の馬, 再構成の馬)]。"""
    rec, t = r["recorded"], r["t40"]
    out = []
    if rec["axis"] != t["axis"]:
        out.append(("軸", [rec["axis"]], [t["axis"]]))
    rp, tp = set(rec["partners"]) - {t["axis"]}, set(t["partners"]) - {rec["axis"]}
    if _partners_differ(rec, t):
        out.append(("相手", sorted(rp - tp), sorted(tp - rp)))
    if sorted(rec.get("konsen_band") or []) != _band(t):
        out.append(("混戦", sorted(rec.get("konsen_band") or []), _band(t)))
    return out


def classify(r, kind, rec_side, t_side, drift_pt):
    """食い違い 1 件の原因の分類（#724 計画の改訂 M5）。

    (a) スナップショットの時点: T-40 のスナップショットが初回スイープの保存でない（遅れがスイープ間隔を超える）。
    (b)/(c) 入力の差: 当時の live と今の再構成で、統計の取り込み（as-of）とコードが違う。入れ替わった馬どうしの
        勝率の差が、その日の「記録の軸の勝率」のずれ（drift_pt）以内なら、入力の差で説明できる。
        (b) と (c) は 1 件ずつには分けられない（当時のコードで再実行しないと切り分けられない）。
    説明できないものは「未分類」。
    """
    if t40_lag_sec(r) > MAX_T40_LAG_SEC:
        return "(a) 時点"
    w = dict((int(n), p * 100) for n, p in r["t40_blended_win"])
    if kind == "混戦" or not rec_side or not t_side:
        return "未分類"
    # 記録の選定に戻すのに要る差（再構成で上に来た馬の最大 − 記録の馬の最小）。2 頭以上の入れ替わりでは最も遠い組
    gap = abs(max(w.get(h, 0) for h in t_side) - min(w.get(h, 0) for h in rec_side))
    if drift_pt is not None and gap <= drift_pt:
        return f"(b)/(c) 入力の差（勝率の差 {gap:.2f}pt ≤ その日のずれ {drift_pt:.2f}pt）"
    return f"未分類（勝率の差 {gap:.2f}pt）"


def check_recorded(records, axis_prob):
    by = defaultdict(list)
    for r in records:
        if r.get("t40") and not r.get("skip"):
            by[_ts(r["captured_at"]).astimezone(JST).strftime("%m-%d")].append(r)
    lags = sorted(t40_lag_sec(r) for rs in by.values() for r in rs)
    if lags:
        print(f"T-40 の遅れ（スナップショット − 初回スイープ）: 中央 {lags[len(lags) // 2]:.0f} 秒・最大 {lags[-1]:.0f} 秒・"
              f"{MAX_T40_LAG_SEC} 秒超 {sum(1 for x in lags if x > MAX_T40_LAG_SEC)}/{len(lags)}")
    print("| 開催日 | R | 軸の不一致 | 相手の不一致 | 混戦の不一致 | 記録の軸の勝率との差（中央値 / 最大） |")
    print("|---|---|---|---|---|---|")
    lines = []
    for day, rs in sorted(by.items()):
        diffs = []
        for r in rs:
            w = dict((int(n), p * 100) for n, p in r["t40_blended_win"])
            ap = axis_prob.get(r["race_id"])
            if ap and ap[0] in w:
                diffs.append(abs(w[ap[0]] - ap[1]))
        diffs.sort()
        drift = diffs[-1] if diffs else None
        counts = defaultdict(int)
        for r in rs:
            for kind, rec_side, t_side in mismatch(r):
                counts[kind] += 1
                lines.append(f"- {day} {r['race_id']} {kind} 記録 {rec_side} → 再構成 {t_side}: "
                             f"{classify(r, kind, rec_side, t_side, drift)}")
        prob = f"{diffs[len(diffs) // 2]:.3f}pt / {diffs[-1]:.3f}pt" if diffs else "—"
        print(f"| {day} | {len(rs)} | {counts['軸']} | {counts['相手']} | {counts['混戦']} | {prob} |")
    print("\n".join(lines))


def main(argv):
    check, axis_prob_path, paths = False, None, []
    it = iter(argv)
    for a in it:
        if a == "--check-recorded":
            check = True
        elif a == "--axis-prob":
            axis_prob_path = next(it, None)
            if axis_prob_path is None:
                print(__doc__, file=sys.stderr)
                return 2
        else:
            paths.append(a)
    records = []
    for path in paths:
        with open(path, encoding="utf-8") as f:
            records += [json.loads(line) for line in f if line.startswith("{")]
    if check:
        check_recorded(records, load_axis_prob(axis_prob_path) if axis_prob_path else {})
        return 0

    # キャッシュは gitignore の作業ファイルなので、worktree から primary のキャッシュを読むときは環境変数で指す
    cache_dir = os.environ.get("NK_RESULT_CACHE_DIR", nk.RESULT_CACHE_DIR)
    races, dropped = build_races(records, lambda rid: read_cached_result(cache_dir, rid))
    print(f"読み込み {len(records)}R・計測 {len(races)}R・除外 {dropped}")
    report([r for r in races if r["day"] != HOLDOUT_DAY], f"主判定（{HOLDOUT_DAY} を除く）")
    report(races, "全日")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
