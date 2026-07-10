#!/usr/bin/env python3
"""late money (オッズの締まり方) シグナルの予備検証 — issue #315 第一関門。

DB の race_odds_snapshots(win) から「朝→直前」の単勝 log-odds drift を算出し、
netkeiba 結果ページの着順と突合して、drift が着順/勝敗を予測するかを見る。

肝: 最終 snapshot が示す市場勝率(=クロージングライン)を baseline に据え、
    drift が「その水準を超えて」着順を説明するか(=残差予測力)を測る。
    これがゼロなら「動いた後には妙味なし」で late money は執行エッジにならない。

依存: psql (PADDOCK_DB_URL), 標準ライブラリ + 同ディレクトリの nk.py（実証済み netkeiba ヘルパ）。
結果着順はローカル JSON キャッシュ。再実行可能。snapshot が増えたら母数が自動で増える。
"""
import os
import re
import sys
import json
import math
import time
import calendar
import subprocess
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # nk.py を同ディレクトリから import
import nk  # noqa: E402  実証済み netkeiba ヘルパ（curl/decode/fetch_result/VENUES）を再利用

# 他ハーネス（gen_predictions.py 等）と同じく host は localhost を避け 127.0.0.1 を使う
# （localhost だと psql 接続が間欠失敗する既知事象）。
DB = os.environ.get("PADDOCK_DB_URL", "postgres://paddock:paddock@127.0.0.1:5432/paddock")
CACHE = os.path.join(os.path.dirname(__file__), ".cache_nk_results")
os.makedirs(CACHE, exist_ok=True)

# slug → JRA 場コード（nk.VENUES: code→(slug,jp) の逆写像。場コード表を nk.py に一元化）
SLUG2CODE = {slug: code for code, (slug, _jp) in nk.VENUES.items()}

MIN_TIMEPOINTS = 2       # drift を測るのに最低2時点
MIN_SPAN_MIN = 30        # 朝→直前とみなす最小スパン(分)


def slug_to_netkeiba_id(slug):
    """'2026-1-hakodate-5-1R' -> '202602010501'"""
    m = re.match(r"^(\d{4})-(\d+)-([a-z]+)-(\d+)-(\d+)R$", slug)
    if not m:
        return None
    year, kai, venue, day, r = m.groups()
    if venue not in SLUG2CODE:
        return None
    return f"{year}{SLUG2CODE[venue]}{int(kai):02d}{int(day):02d}{int(r):02d}"


def fetch_snapshots():
    """win snapshot を (race_id, horse_num, odds, fetched_at) で取得。"""
    sql = (
        "SELECT race_id, combination_key, odds, fetched_at "
        "FROM race_odds_snapshots WHERE bet_type='win' ORDER BY race_id, combination_key, fetched_at"
    )
    out = subprocess.run(
        ["psql", DB, "-At", "-F", "\t", "-c", sql],
        capture_output=True, text=True, check=True,
    ).stdout
    rows = []
    for line in out.splitlines():
        rid, num, odds, fetched = line.split("\t")
        rows.append((rid, int(num), float(odds), fetched))
    return rows


def parse_ts(s):
    # '2026-06-27T00:01:03.024586+00:00' -> epoch 秒 (UTC 保存前提・相対差専用。うるう秒無視)。
    m = re.match(r"(\d{4})-(\d\d)-(\d\d)T(\d\d):(\d\d):(\d\d)", s)
    y, mo, d, h, mi, se = map(int, m.groups())
    return calendar.timegm((y, mo, d, h, mi, se, 0, 0, 0))


def fetch_finish(nk_id):
    """{umaban: finishing_position} を返す。除外/中止(着順 None)は除く。

    パースは nk.fetch_result（実証済み・枠番/馬番の誤検出対策と空警告つき）に委譲し、
    抽出済み着順を JSON でローカルキャッシュ（再走で refetch しない・netkeiba への礼儀）。"""
    path = os.path.join(CACHE, f"{nk_id}.json")
    if os.path.exists(path):
        return {int(k): v for k, v in json.load(open(path, encoding="utf-8")).items()}
    rows = nk.fetch_result(nk_id)  # curl+parse。取得成功だが 0 行なら nk 側が warn
    finish = {r["horse_num"]: r["rank"] for r in rows if r["rank"] is not None}
    with open(path, "w", encoding="utf-8") as f:
        json.dump(finish, f)
    time.sleep(1.5)  # netkeiba への礼儀 (未キャッシュ取得時のみ)
    return finish


# ---------- 統計ヘルパ (stdlib のみ) ----------
def rankdata(xs):
    order = sorted(range(len(xs)), key=lambda i: xs[i])
    ranks = [0.0] * len(xs)
    i = 0
    while i < len(xs):
        j = i
        while j + 1 < len(xs) and xs[order[j + 1]] == xs[order[i]]:
            j += 1
        avg = (i + j) / 2.0 + 1.0
        for k in range(i, j + 1):
            ranks[order[k]] = avg
        i = j + 1
    return ranks


def pearson(a, b):
    n = len(a)
    ma, mb = sum(a) / n, sum(b) / n
    num = sum((a[i] - ma) * (b[i] - mb) for i in range(n))
    da = math.sqrt(sum((x - ma) ** 2 for x in a))
    db = math.sqrt(sum((x - mb) ** 2 for x in b))
    return num / (da * db) if da and db else float("nan")


def spearman(a, b):
    return pearson(rankdata(a), rankdata(b))


def main():
    rows = fetch_snapshots()
    by_race = defaultdict(lambda: defaultdict(list))  # race -> horse -> [(ts,odds)]
    for rid, num, odds, fetched in rows:
        if odds <= 0:
            continue
        by_race[rid][num].append((parse_ts(fetched), odds))

    horses = []  # dict per horse
    used_races = 0
    skipped_span = 0
    no_result = 0
    for rid, hs in by_race.items():
        # 時点数・スパン判定はレース全体の fetched_at 集合で
        all_ts = sorted({t for h in hs.values() for (t, _) in h})
        if len(all_ts) < MIN_TIMEPOINTS:
            continue
        span_min = (all_ts[-1] - all_ts[0]) / 60.0
        if span_min < MIN_SPAN_MIN:
            skipped_span += 1
            continue
        nk_id = slug_to_netkeiba_id(rid)
        if not nk_id:
            continue
        try:
            finish = fetch_finish(nk_id)
        except Exception as e:
            print(f"  ! {rid} ({nk_id}) 取得失敗: {e}", file=sys.stderr)
            no_result += 1
            continue
        if not finish:
            no_result += 1
            continue

        # 各馬 first/last odds
        rec = []
        for num, seq in hs.items():
            seq = sorted(seq)
            o_first, o_last = seq[0][1], seq[-1][1]
            rec.append((num, o_first, o_last))
        # レース内で最終オッズから implied 正規化 (控除除去)
        s_last = sum(1.0 / o_last for (_, _, o_last) in rec)
        used = False
        for num, o_first, o_last in rec:
            if num not in finish:
                continue
            drift = math.log(o_first) - math.log(o_last)  # >0: 締まった(金が入った)
            p_last = (1.0 / o_last) / s_last
            fin = finish[num]
            horses.append({
                "race": rid, "num": num, "o_first": o_first, "o_last": o_last,
                "drift": drift, "p_last": p_last,
                "finish": fin, "won": 1 if fin == 1 else 0,
                "placed": 1 if fin <= 3 else 0,
                "resid_win": (1 if fin == 1 else 0) - p_last,
            })
            used = True
        if used:
            used_races += 1

    print("=" * 70)
    print(f"対象レース(結果取得済): {used_races}  / span不足でskip: {skipped_span} / 結果無: {no_result}")
    print(f"対象馬(頭数): {len(horses)}")
    if len(horses) < 30:
        print("サンプル過少。中断。")
        return

    drift = [h["drift"] for h in horses]
    finish = [h["finish"] for h in horses]
    won = [h["won"] for h in horses]
    resid = [h["resid_win"] for h in horses]
    p_last = [h["p_last"] for h in horses]

    print("\n--- (1) 生の相関 (level未制御) ---")
    print(f"Spearman(drift, finish) = {spearman(drift, finish):+.3f}  (負=締まる馬ほど好走)")
    print(f"Spearman(drift, won)    = {spearman(drift, won):+.3f}  (正=締まる馬ほど勝つ)")
    print(f"Spearman(p_last, finish)= {spearman(p_last, finish):+.3f}  (参考:終値人気の効き)")

    print("\n--- (2) level制御: drift は終値の残差を説明するか ---")
    print(f"Spearman(drift, resid_win) = {spearman(drift, resid):+.3f}")
    print("  resid_win = 実勝敗(0/1) - 終値implied勝率。>0相関なら終値を超える予測力あり。")

    print("\n--- (3) drift 五分位: 終値水準を揃えて勝率/残差を比較 ---")
    idx = sorted(range(len(horses)), key=lambda i: drift[i])
    q = len(idx) // 5
    print(f"{'quintile':>8} {'n':>4} {'drift平均':>9} {'終値implied':>10} {'実勝率':>7} {'残差(実-implied)':>14} {'複勝率':>7}")
    for k in range(5):
        seg = idx[k * q:(k + 1) * q] if k < 4 else idx[k * q:]
        n = len(seg)
        md = sum(drift[i] for i in seg) / n
        mp = sum(p_last[i] for i in seg) / n
        wr = sum(won[i] for i in seg) / n
        pr = sum(horses[i]["placed"] for i in seg) / n
        rr = sum(resid[i] for i in seg) / n
        tag = "最も緩んだ" if k == 0 else ("最も締まった" if k == 4 else "")
        print(f"Q{k+1:<7} {n:>4} {md:>+9.3f} {mp:>10.3f} {wr:>7.3f} {rr:>+14.4f} {pr:>7.3f}  {tag}")

    print("\n--- (4) logloss: 終値のみ vs drift 加味 (簡易ロジスティック1変数) ---")
    # baseline: p_last をそのまま採用。 alt: logit(p_last) + b*drift を b をグリッド探索で最尤化。
    def logloss(ps):
        s = 0.0
        for i, h in enumerate(horses):
            p = min(max(ps[i], 1e-6), 1 - 1e-6)
            s += -(h["won"] * math.log(p) + (1 - h["won"]) * math.log(1 - p))
        return s / len(horses)

    base_ll = logloss(p_last)
    best_b, best_ll = 0.0, base_ll
    for b10 in range(-30, 31):
        b = b10 / 10.0
        ps = []
        for h in horses:
            lo = math.log(h["p_last"] / (1 - h["p_last"])) + b * h["drift"]
            ps.append(1 / (1 + math.exp(-lo)))
        # 再正規化はしない(頭数バランス無視の近似)。単変数の効きの符号確認が目的。
        ll = logloss(ps)
        if ll < best_ll:
            best_ll, best_b = ll, b
    print(f"baseline(終値のみ) logloss = {base_ll:.4f}")
    print(f"best drift係数 b={best_b:+.2f} で logloss = {best_ll:.4f}  (改善 {base_ll-best_ll:+.4f})")
    print("  注: in-sample・非正規化の近似。改善が微小/b≈0なら drift の追加情報は乏しい。")


if __name__ == "__main__":
    main()
