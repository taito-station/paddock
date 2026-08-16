#!/usr/bin/env python3
"""ROI ゲート（参考ROI ≥ 100%）の較正測定（#571。#249 のバケット較正を統合）.

`predict-watch` が出す参考ROIは ADR 0055（EV 層分離）以降、**軸/相手は blended（α=0.2）で選び
EV は pure（α=1.0）で評価する** 2 系統構成になった。この定義に対しゲートは 100% に置かれたままだが、
現行定義での較正は一度も測られていない（ADR 0044 / 0045 の 71R 検証は**分離前**の EV 定義）。

本ハーネスは `live_ev_snapshots`（predict-watch が実際に記録した判定ROIと買い目伝票）を
netkeiba 確定払戻で精算し、**判定ROI と実現ROI の関係**を測る。

【second source を作らない】買い目は Rust `build_portfolio` が組んだ伝票そのもの（`slip`）を使い、
Python 側で組み直さない（ADR 0064 が警告する買い方ロジックの二重実装を避ける）。本スクリプトの
責務は「記録済みの伝票を確定払戻で精算して集計する」ことだけ。

【循環回避】判定に使ったオッズ（盤面）と清算（netkeiba 実払戻）を分離する。同一オッズで清算すると
「EV が高い脚は当たれば必ずその倍率返る」恒真化で較正が不当に良く見える（ADR 0041 / 0044 と同方針）。

【市場整合ROI】各券種の全組オッズから market-implied 確率を作り、**同じ伝票**を市場確率で評価する。
的中組数 W（馬連/3連複=1・ワイド=3）で正規化すると、これは任意の伝票について 1−控除率 に一致する。
よって `判定ROI ÷ 市場整合ROI` が「pure モデルが選ばれた脚を市場の何倍で値付けしているか」を直接与える。

使い方:
    # 払戻は fetch_payouts.py で日ごとに取得済みとする（payouts_YYYYMMDD.json）
    python3 gate_calibration.py --payouts-dir /path/to/payouts

    # 期間を絞る / バケット境界を変える
    python3 gate_calibration.py --payouts-dir DIR --from 2026-07-11 --to 2026-08-09 \
        --buckets 20,40,60,80
"""
import argparse
import glob
import json
import os
import random
import re
import subprocess
import sys
from collections import defaultdict
from datetime import datetime, timedelta, timezone
from pathlib import Path

from odds_guard import is_payout_odds
from umaren_backtest import spearman

JST = timezone(timedelta(hours=9))

# 券種ごとの「的中する組の数」。market-implied 確率の正規化に使う（Σq = WIN_COMBOS）。
# 馬連・3連複は 1 組だけが的中。ワイドは 3 着以内の 2 頭組が 3 通り的中する。
WIN_COMBOS = {"quinella": 1, "trio": 1, "wide": 3}
BET_LABEL = {"wide": "ワイド", "quinella": "馬連", "trio": "3連複"}


# --- 組番キー ---------------------------------------------------------------
def combo_key(combo):
    """馬番リスト → 払戻の combination_code（無順券種: 昇順ソートして `-` 連結）。

    本体 Rust `BetCombination::combination_code` および `nk.fetch_payouts` の規則と一致させる。
    ソートは**数値順**（文字列順だと 10 < 5 になり 3 連複のキーが崩れる）。
    """
    return "-".join(str(n) for n in sorted(int(n) for n in combo))


# --- 時刻 -------------------------------------------------------------------
def to_jst(captured_at):
    """`live_ev_snapshots.captured_at`（UTC の ISO8601 文字列）を JST の datetime へ。

    末尾 `Z` は `fromisoformat` が古い Python で解釈できないため `+00:00` に正規化する。
    タイムゾーン指定が無い文字列は UTC とみなす（本テーブルは UTC で書かれる）。
    """
    s = captured_at.strip().replace("Z", "+00:00")
    dt = datetime.fromisoformat(s)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt.astimezone(JST)


def post_dt(date, post_time):
    """`date`(YYYY-MM-DD) + `post_time`(HH:MM, JST) → JST の発走 datetime。不正値は None。"""
    if not date or not post_time:
        return None
    m = re.fullmatch(r"([0-9]{1,2}):([0-9]{2})", post_time.strip())
    if not m or not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", date.strip()):
        return None
    y, mo, d = (int(x) for x in date.split("-"))
    return datetime(y, mo, d, int(m.group(1)), int(m.group(2)), tzinfo=JST)


# --- レース単位の代表スイープ -------------------------------------------------
def pick_race_rows(rows):
    """1 レースのスイープ行から `final`（発走前の最終）と `ever`（全スイープ最大 ROI）を選ぶ。

    `snapshot_ev_report.py` の ever/final 規約に合わせる。発走時刻が取れない、または発走前の
    スイープが 1 本も無いレースは final=None（集計から落とし、呼び出し側が件数を報告する）。
    `ever` は発走前スイープのみから採る（発走後の行は判断材料になり得ないため）。
    """
    if not rows:
        return None, None
    post = post_dt(rows[0]["date"], rows[0]["post_time"])
    if post is None:
        return None, None
    pre = [r for r in rows if to_jst(r["captured_at"]) <= post]
    if not pre:
        return None, None
    final = max(pre, key=lambda r: to_jst(r["captured_at"]))
    ever = max(pre, key=lambda r: r["roi"])
    return final, ever


# --- 精算 -------------------------------------------------------------------
def settle(legs, payouts):
    """伝票 legs を確定払戻で精算し (stake, ret, hit_legs, by_type) を返す。

    払戻は 100 円あたりの円なので `払戻 = amount/100 × payout`。払戻辞書に無い組＝不的中で 0 円。
    `amount == 0` の脚（券種予算を賄えず ¥0 になった脚）は賭けていないので分母にも入れない。
    同着で複数組が的中する場合も、払戻辞書が該当組を持つため自然に拾える。
    """
    stake = 0.0
    ret = 0.0
    hit_legs = 0
    by_type = defaultdict(lambda: {"stake": 0.0, "ret": 0.0, "legs": 0, "hits": 0})
    for leg in legs:
        amount = float(leg.get("amount") or 0)
        if amount <= 0:
            continue
        bt = leg.get("bet_type")
        key = combo_key(leg.get("combo") or [])
        pay = float((payouts.get(bt) or {}).get(key, 0) or 0)
        r = amount / 100.0 * pay
        stake += amount
        ret += r
        by_type[bt]["stake"] += amount
        by_type[bt]["ret"] += r
        by_type[bt]["legs"] += 1
        if pay > 0:
            hit_legs += 1
            by_type[bt]["hits"] += 1
    return stake, ret, hit_legs, dict(by_type)


# --- 配分の入れ替え（#600 / ADR 0080） ----------------------------------------
# 券種の並びは Rust `PortfolioConfig.alloc` と同じ (連系ペア, ワイド, 三連複)。
ALLOC_TYPES = ("quinella", "wide", "trio")


def distribute(type_budget, n):
    """券種予算を n 点へ 100 円単位で均等配分する。Rust `portfolio::distribute` の鏡映。

    全点に ¥100 すら置けないときは賄える点数ぶんだけ ¥100 を置く（残りは ¥0＝買わない）。
    **買い方ロジックの second source を作らないため、脚の選定には一切触れない**——
    ここでやるのは「同じ脚に別の金額を置いたらどうだったか」の再計算だけ（ADR 0064）。
    """
    if n <= 0 or type_budget < 100:
        return [0] * max(n, 0)
    per = type_budget // n // 100 * 100
    if per >= 100:
        return [per] * n
    affordable = min(type_budget // 100, n)
    return [100] * affordable + [0] * (n - affordable)


def realloc_and_settle(legs, payouts, alloc, race_budget):
    """記録済みの脚をそのままに、配分だけ変えて確定払戻で再精算する。

    `alloc` は (連系ペア, ワイド, 三連複) の相対重み。Rust `build_portfolio` と同じ
    「券種予算を 100 円単位に floor → 券種内を 100 円単位で均等配分」の 2 段 floor を通す。
    返り値は (stake, ret, by_type)。**混戦（box 脚を含む）レースは対象外**——
    混戦は `KONSEN_ALLOC`（4 レイヤー）で組まれており 3 要素の alloc では再現できない。
    """
    by_type = defaultdict(list)
    for leg in legs:
        by_type[leg.get("bet_type")].append(leg)
    total_w = sum(alloc)
    if total_w <= 0:
        return 0.0, 0.0, {}
    stake = ret = 0.0
    per_type = {}
    for bt, w in zip(ALLOC_TYPES, alloc):
        group = by_type.get(bt) or []
        type_budget = race_budget * w // total_w // 100 * 100
        s = r = 0.0
        for leg, amount in zip(group, distribute(type_budget, len(group))):
            if amount <= 0:
                continue
            pay = float((payouts.get(bt) or {}).get(combo_key(leg.get("combo") or []), 0) or 0)
            s += amount
            r += amount / 100.0 * pay
        stake += s
        ret += r
        per_type[bt] = (s, r)
    return stake, ret, per_type


def parse_alloc(text):
    """`1500,1500,2000` を (連系ペア, ワイド, 三連複) の重みタプルへ。"""
    parts = [p.strip() for p in text.split(",")]
    if len(parts) != 3:
        raise ValueError(f"--compare-alloc は 3 つの重みをカンマ区切りで指定する（馬連,ワイド,3連複）: {text!r}")
    try:
        weights = tuple(int(p) for p in parts)
    except ValueError as e:
        raise ValueError(f"--compare-alloc の重みは整数で指定する: {text!r}") from e
    if any(w < 0 for w in weights) or sum(weights) <= 0:
        raise ValueError(f"--compare-alloc の重みは非負で合計が正であること: {text!r}")
    return weights


# --- 市場整合 ROI -------------------------------------------------------------
def market_fair_roi(legs, odds_by_type):
    """同じ伝票を market-implied 確率で評価した ROI（＝おおむね 1−控除率）。

    `odds_by_type`: {bet_type: {combination_key: odds}}（ワイドは low/high の mid を入れておく）。
    各券種内で `q_i = (1/O_i) × W / Σ_j(1/O_j)`（W=的中組数）と正規化するので、
    どんな買い方でも `Σ s_i q_i O_i / Σ s_i = W/Σ_j(1/O_j)` ＝ 1−控除率 に一致する。
    オッズ表を欠く券種の脚は評価不能なので分母から落とす（全滅なら None）。
    """
    # Σ(1/O) は券種ごとに一度だけ求める（脚ごとに数千組を舐め直さない）。
    inv_sums = {bt: sum(1.0 / v for v in table.values() if v and v > 0)
                for bt, table in odds_by_type.items()}
    stake = 0.0
    exp = 0.0
    for leg in legs:
        amount = float(leg.get("amount") or 0)
        if amount <= 0:
            continue
        bt = leg.get("bet_type")
        table = odds_by_type.get(bt) or {}
        o = table.get(combo_key(leg.get("combo") or []))
        inv_sum = inv_sums.get(bt, 0.0)
        if not o or o <= 0 or inv_sum <= 0:
            continue
        q = (1.0 / o) * WIN_COMBOS.get(bt, 1) / inv_sum
        stake += amount
        exp += amount * q * o
    return (exp / stake) if stake > 0 else None


# --- 集計 -------------------------------------------------------------------
def pooled_roi(races):
    """プール実現ROI（Σ払戻 / Σ賭金）を % で返す。賭金ゼロなら None。"""
    s = sum(r["stake"] for r in races)
    return (sum(r["ret"] for r in races) / s * 100.0) if s > 0 else None


def bootstrap_ci(races, iters=2000, seed=571, lo=2.5, hi=97.5):
    """レース単位のリサンプリングでプール実現ROIの信頼区間を出す。

    実現ROIは少数の高配当的中に強く依存する（#571 が n=12 で指摘した点）。点推定だけを出すと
    その依存が見えないので、必ず併記する。seed 固定で再現可能。
    """
    if len(races) < 2:
        return None, None
    rnd = random.Random(seed)
    vals = []
    n = len(races)
    for _ in range(iters):
        sample = [races[rnd.randrange(n)] for _ in range(n)]
        v = pooled_roi(sample)
        if v is not None:
            vals.append(v)
    if not vals:
        return None, None
    vals.sort()
    def pct(p):
        i = min(len(vals) - 1, max(0, int(round(p / 100.0 * (len(vals) - 1)))))
        return vals[i]
    return pct(lo), pct(hi)


def bucketize(races, edges):
    """判定ROI でレースをバケット分けする。edges=[20,40,60,80] → 5 帯。"""
    edges = sorted(edges)
    buckets = [[] for _ in range(len(edges) + 1)]
    for r in races:
        i = 0
        while i < len(edges) and r["judged_roi"] >= edges[i]:
            i += 1
        buckets[i].append(r)
    labels = []
    for i in range(len(edges) + 1):
        if i == 0:
            labels.append(f"<{edges[0]:g}%")
        elif i == len(edges):
            labels.append(f"≥{edges[-1]:g}%")
        else:
            labels.append(f"{edges[i-1]:g}–{edges[i]:g}%")
    return labels, buckets


def threshold_table(races, grid):
    """判定ROI ≥ θ で絞ったときの n と実現ROI（＝(a) 案の較正済み閾値の読み取り表）。"""
    out = []
    for th in grid:
        sub = [r for r in races if r["judged_roi"] >= th]
        out.append((th, len(sub), pooled_roi(sub)))
    return out


# --- 入力ロード -------------------------------------------------------------
_LIVE_COLS = ["race_id", "date", "venue", "race_no", "post_time", "captured_at",
              "roi", "axis", "konsen", "odds_missing", "slip"]


def _check_date(d):
    # [0-9] に固定（\d は Unicode 数字も通す）。SQL へ補間するため呼び出し側検証に依存しない。
    if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", d):
        raise ValueError(f"日付は YYYY-MM-DD のみ許可: {d!r}")
    return d


def psql_dump_live_ev(db_url, date_from, date_to):
    """期間内の live_ev_snapshots を TSV 文字列で返す（slip は 1 行 JSON）。"""
    _check_date(date_from)
    _check_date(date_to)
    sql = (
        "SELECT race_id, date, venue, race_no, COALESCE(post_time,''), captured_at, "
        "       roi, axis, konsen, odds_missing, slip::text "
        "FROM live_ev_snapshots "
        f"WHERE date BETWEEN '{date_from}' AND '{date_to}' "
        "ORDER BY race_id, captured_at;"
    )
    out = subprocess.run(["psql", db_url, "-tA", "-F", "\t", "-c", sql],
                         capture_output=True, text=True, check=True)
    return out.stdout


def load_live_ev(tsv_text):
    """live_ev TSV → race_id ごとのスイープ行 list。"""
    by_race = defaultdict(list)
    for line in tsv_text.splitlines():
        if not line.strip():
            continue
        cells = line.split("\t")
        if len(cells) != len(_LIVE_COLS):
            print(f"[warn] live_ev の想定外の列数 {len(cells)} をスキップ: {line[:80]}",
                  file=sys.stderr)
            continue
        row = dict(zip(_LIVE_COLS, cells))
        try:
            row["roi"] = float(row["roi"])
            row["race_no"] = int(row["race_no"])
            row["axis"] = int(row["axis"])
            row["slip"] = json.loads(row["slip"])
        except (ValueError, json.JSONDecodeError) as e:
            print(f"[warn] live_ev の行を解釈できずスキップ ({e}): {line[:80]}", file=sys.stderr)
            continue
        row["konsen"] = row["konsen"] == "t"
        row["odds_missing"] = row["odds_missing"] == "t"
        by_race[row["race_id"]].append(row)
    return by_race


def psql_dump_odds(db_url, date_from, date_to):
    """期間内の race_odds_snapshots（連系 3 券種）を TSV で返す（市場整合ROI 診断用）。"""
    _check_date(date_from)
    _check_date(date_to)
    bet_in = ",".join(f"'{t}'" for t in sorted(WIN_COMBOS))
    sql = (
        "SELECT s.race_id, s.bet_type, s.combination_key, s.odds, "
        "       COALESCE(s.odds_high::text,''), s.fetched_at "
        "FROM race_odds_snapshots s JOIN race_cards c ON c.race_id = s.race_id "
        f"WHERE c.date BETWEEN '{date_from}' AND '{date_to}' "
        f"  AND s.bet_type IN ({bet_in}) "
        "ORDER BY s.race_id, s.fetched_at;"
    )
    out = subprocess.run(["psql", db_url, "-tA", "-F", "\t", "-c", sql],
                         capture_output=True, text=True, check=True)
    return out.stdout


def load_odds(tsv_text):
    """オッズ TSV → {race_id: {fetched_at: {bet_type: {combo_key: odds}}}}。

    ワイドは low/high の帯で保存されるので mid=(low+high)/2 を採る（`live_ev.py` のワイド意味論）。
    """
    out = defaultdict(lambda: defaultdict(lambda: defaultdict(dict)))
    for line in tsv_text.splitlines():
        if not line.strip():
            continue
        cells = line.split("\t")
        if len(cells) != 6:
            print(f"[warn] odds の想定外の列数 {len(cells)} をスキップ: {line[:80]}", file=sys.stderr)
            continue
        rid, bt, key, odds, odds_high, fetched = cells
        try:
            low = float(odds)
            # 番兵（99999.9 等）は払戻倍率ではないので採用しない（#621）。band は**中点化の前**に
            # 見る——中点にすると番兵と実値の平均になって検知できなくなる。
            if not is_payout_odds(low):
                continue
            o = low
            if odds_high:
                high = float(odds_high)
                if not is_payout_odds(high):
                    continue
                o = (low + high) / 2.0
        except ValueError:
            continue
        if o <= 0:
            continue
        out[rid][fetched][bt][key] = o
    return out


def nearest_odds(odds_for_race, captured_at):
    """captured_at 以前で最も近い fetched_at のオッズ表を返す（無ければ最古を返す）。"""
    if not odds_for_race:
        return {}
    target = to_jst(captured_at)
    keyed = sorted(odds_for_race.items(), key=lambda kv: to_jst(kv[0]))
    best = None
    for fetched, table in keyed:
        if to_jst(fetched) <= target:
            best = table
    return best if best is not None else keyed[0][1]


def load_payouts_dir(path):
    """payouts_YYYYMMDD.json 群 → {(date, venue_jp, race_num): {bet_type: {key: payout}}}。

    ファイル名から日付を採る（fetch_payouts.py の出力は各レースに日付を持たないため）。
    """
    out = {}
    files = sorted(glob.glob(os.path.join(path, "payouts_*.json")))
    if not files:
        raise FileNotFoundError(f"payouts_YYYYMMDD.json が見つからない: {path}")
    for f in files:
        m = re.search(r"payouts_([0-9]{8})\.json$", os.path.basename(f))
        if not m:
            print(f"[warn] 日付を読めないファイルをスキップ: {f}", file=sys.stderr)
            continue
        ymd = m.group(1)
        date = f"{ymd[0:4]}-{ymd[4:6]}-{ymd[6:8]}"
        for entry in json.loads(Path(f).read_text()):
            out[(date, entry["venue_jp"], int(entry["race_num"]))] = entry.get("payouts") or {}
    return out


# --- レポート ---------------------------------------------------------------
def _fmt(v, suffix="%"):
    return "—" if v is None else f"{v:.1f}{suffix}"


def axis_stability(by_race):
    """スイープ間で軸（◎）が入れ替わったレースを数える。

    軸は `rank_probs`（市場ブレンド α=0.2）で選ばれるので、オッズが動くと軸も動きうる。
    CLAUDE.md「軸ロックとズレ増額」（REQ-D01-003 / ADR 0060）は軸をブラさないと定めているため、
    フリップが起きているなら判定ROIは「同じ買い目の値付けの変化」を測っていないことになる。
    発走前スイープのみを見る（発走後の再計算は判断材料にならない）。
    """
    multi = 0
    flipped = []
    for rid, rows in sorted(by_race.items()):
        post = post_dt(rows[0]["date"], rows[0]["post_time"])
        if post is None:
            continue
        pre = sorted((r for r in rows if to_jst(r["captured_at"]) <= post),
                     key=lambda r: to_jst(r["captured_at"]))
        if len(pre) < 2:
            continue
        multi += 1
        # 出現順に重複を畳んだ軸の遷移（1→9→1 のような往復を「軸 2 種」で潰さない）。
        seq = [r["axis"] for r in pre]
        path = [a for i, a in enumerate(seq) if i == 0 or a != seq[i - 1]]
        if len(set(seq)) >= 2:
            flipped.append((pre[0], pre[-1], path))
    return multi, flipped


def report_alloc_comparison(races, allocs, out=sys.stdout):
    """券種配分を入れ替えたときの実現ROI・1レース賭金を並べる（#600 / ADR 0080）。

    脚（どの組番を買ったか）は記録どおりで動かさず、**金額だけ**を再計算する。
    比較の基準は「記録どおり」＝実際に張られた金額で、これが現行実装の挙動そのもの。
    混戦レースは `KONSEN_ALLOC`（4 レイヤー）で組まれており 3 要素の alloc では
    再現できないので母集団から外し、その件数を明示する（黙って落とすと母数が食い違う）。
    """
    p = lambda *a: print(*a, file=out)
    target = [r for r in races if not r["has_box"] and r["race_budget"]]
    excluded = len(races) - len(target)
    p(f"\n=== 券種配分の比較（非混戦 {len(target)} レース"
      f"{f'・混戦/予算不明 {excluded} レースを除外' if excluded else ''}）===")
    if not target:
        p("  比較対象なし")
        return
    p(f"{'配分 (馬連,ワイド,3連複)':>26} | {'賭金':>10} | {'払戻':>10} | {'実現ROI':>8} | {'1R平均':>8}")
    p("-" * 76)

    def line(label, stake, ret):
        roi = f"{ret / stake * 100:.1f}%" if stake > 0 else "—"
        avg = f"¥{stake / len(target):,.0f}" if target else "—"
        p(f"{label:>26} | ¥{stake:>9,.0f} | ¥{ret:>9,.0f} | {roi:>8} | {avg:>8}")

    # 記録どおり（＝実際に張られた金額）。
    line("記録どおり", sum(r["stake"] for r in target), sum(r["ret"] for r in target))
    for alloc in allocs:
        stake = ret = 0.0
        for r in target:
            s, t, _ = realloc_and_settle(r["legs"], r["payouts"], alloc, r["race_budget"])
            stake += s
            ret += t
        line(",".join(str(w) for w in alloc), stake, ret)
    p("  ※ 脚（どの組番を買うか）は記録どおりで固定し、金額だけ入れ替えている。")


def report_axis_stability(by_race, out=sys.stdout):
    from nk import SLUG2JP
    p = lambda *a: print(*a, file=out)
    multi, flipped = axis_stability(by_race)
    p(f"\n=== 軸（◎）の安定性 — 発走前に 2 スイープ以上あった {multi} レース ===")
    if not multi:
        return
    p(f"スイープ間で軸が入れ替わったレース: {len(flipped)}/{multi}（{len(flipped)/multi*100:.0f}%）")
    p("  軸は rank_probs（市場ブレンド α=0.2）で選ぶためオッズと一緒に動く。")
    p("  CLAUDE.md 軸ロック（REQ-D01-003 / ADR 0060）は軸を動かさないと定めている。")
    for first, last, path in flipped[:10]:
        venue = SLUG2JP.get(first["venue"], first["venue"])
        p(f"    {first['date']} {venue}{first['race_no']:>2}R"
          f"  軸 {'→'.join(str(a) for a in path)}"
          f"  (ROI {first['roi']:.1f}% → {last['roi']:.1f}%)")
    if len(flipped) > 10:
        p(f"    …ほか {len(flipped) - 10} レース")


def report(races, edges, out=sys.stdout):
    p = lambda *a: print(*a, file=out)
    n = len(races)
    stake = sum(r["stake"] for r in races)
    ret = sum(r["ret"] for r in races)
    lo, hi = bootstrap_ci(races)
    p(f"\n=== 全体（n={n}） ===")
    p(f"賭金 ¥{stake:,.0f} / 払戻 ¥{ret:,.0f} / 実現ROI {_fmt(pooled_roi(races))}"
      f"  [bootstrap 95% CI {_fmt(lo)} 〜 {_fmt(hi)}]")
    p(f"判定ROI 平均 {sum(r['judged_roi'] for r in races)/n:.1f}%"
      f" / 最小 {min(r['judged_roi'] for r in races):.1f}%"
      f" / 最大 {max(r['judged_roi'] for r in races):.1f}%")
    mf = [r["market_fair"] for r in races if r["market_fair"] is not None]
    if mf:
        avg_mf = sum(mf) / len(mf) * 100.0
        avg_j = sum(r["judged_roi"] for r in races if r["market_fair"] is not None) / len(mf)
        p(f"市場整合ROI 平均 {avg_mf:.1f}%（n={len(mf)}）"
          f" → 判定ROI ÷ 市場整合ROI = {avg_j/avg_mf:.2f}")
    hit_races = sum(1 for r in races if r["ret"] > 0)
    p(f"的中レース {hit_races}/{n}（{hit_races/n*100:.0f}%）")

    # 予算執行率: 券種予算と脚単価の 2 段階 100 円切り捨てで、伝票が予算に届かないことがある。
    # ROI は比なので結論には効かないが、CLAUDE.md「各レース予算ちょうどに収める」との乖離は報告する。
    budgeted = [r for r in races if r["race_budget"]]
    if budgeted:
        b = sum(r["race_budget"] for r in budgeted)
        s = sum(r["stake"] for r in budgeted)
        p(f"予算執行率 {s/b*100:.1f}%（予算 ¥{b:,.0f} に対し実賭金 ¥{s:,.0f}・n={len(budgeted)}）")

    p("\n=== 較正バケット（判定ROI 帯 → 実現ROI）===")
    p(f"{'帯':>10} | {'n':>4} | {'判定ROI':>8} | {'実現ROI':>9} | {'的中率':>6}")
    p("-" * 52)
    labels, buckets = bucketize(races, edges)
    for label, bucket in zip(labels, buckets):
        if not bucket:
            p(f"{label:>10} | {0:>4} | {'—':>8} | {'—':>9} | {'—':>6}")
            continue
        j = sum(r["judged_roi"] for r in bucket) / len(bucket)
        h = sum(1 for r in bucket if r["ret"] > 0) / len(bucket) * 100
        p(f"{label:>10} | {len(bucket):>4} | {j:>7.1f}% | {_fmt(pooled_roi(bucket)):>9} | {h:>5.0f}%")

    rho = spearman([r["judged_roi"] for r in races], [r["realized_roi"] for r in races])
    p(f"\nSpearman(判定ROI, レース毎実現ROI) = {rho:+.3f}")
    p("  ADR 0045 の定義: 逆予測性の解消 = Spearman ≥ 0 かつ 上位帯の実現ROI ≥ 全体平均")

    p("\n=== 判定ROI ≥ θ で絞ったときの実現ROI（(a) 案の閾値読み取り）===")
    p(f"{'θ':>6} | {'n':>4} | {'実現ROI':>9}")
    p("-" * 26)
    grid = [0, 20, 30, 40, 50, 60, 70, 80, 100]
    for th, cnt, roi in threshold_table(races, grid):
        p(f"{th:>5}% | {cnt:>4} | {_fmt(roi):>9}")

    p("\n=== 券種別（実現）===")
    p(f"{'券種':>8} | {'賭金':>10} | {'払戻':>10} | {'実現ROI':>9} | {'的中脚':>10}")
    p("-" * 60)
    agg = defaultdict(lambda: {"stake": 0.0, "ret": 0.0, "legs": 0, "hits": 0})
    for r in races:
        for bt, v in r["by_type"].items():
            for k in ("stake", "ret", "legs", "hits"):
                agg[bt][k] += v[k]
    for bt in ("wide", "quinella", "trio"):
        v = agg.get(bt)
        if not v or v["stake"] <= 0:
            continue
        p(f"{BET_LABEL[bt]:>8} | ¥{v['stake']:>9,.0f} | ¥{v['ret']:>9,.0f} |"
          f" {v['ret']/v['stake']*100:>8.1f}% | {v['hits']:>4}/{v['legs']:<5}")

    p("\n=== 日別 ===")
    p(f"{'日付':>12} | {'n':>3} | {'判定ROI平均':>10} | {'実現ROI':>9}")
    p("-" * 44)
    by_date = defaultdict(list)
    for r in races:
        by_date[r["date"]].append(r)
    for date in sorted(by_date):
        rs = by_date[date]
        j = sum(x["judged_roi"] for x in rs) / len(rs)
        p(f"{date:>12} | {len(rs):>3} | {j:>9.1f}% | {_fmt(pooled_roi(rs)):>9}")

    top = sorted(races, key=lambda r: -r["realized_roi"])[:5]
    p("\n=== 実現ROI 上位 5（外れ値依存の確認）===")
    for r in top:
        p(f"  {r['date']} {r['venue']}{r['race_no']:>2}R"
          f"  判定 {r['judged_roi']:.1f}% → 実現 {r['realized_roi']:.0f}%"
          f"  (¥{r['stake']:,.0f} → ¥{r['ret']:,.0f})")
    # 同値のレースを巻き込まないよう、値比較ではなく同一性（id）で上位 5 を除く。
    top_ids = {id(r) for r in top}
    rest = [r for r in races if id(r) not in top_ids]
    p(f"  上位 5 を除いた実現ROI: {_fmt(pooled_roi(rest))}（n={len(rest)}）")


# --- main -------------------------------------------------------------------
def build_races(by_race, payouts, odds, use_ever=False):
    """スイープ行・払戻・オッズを突き合わせてレース単位の評価レコードを作る。"""
    from nk import SLUG2JP
    races, skipped = [], defaultdict(int)
    for rid, rows in sorted(by_race.items()):
        final, ever = pick_race_rows(rows)
        if final is None:
            skipped["発走前スイープなし/発走時刻不明"] += 1
            continue
        row = ever if use_ever else final
        venue_jp = SLUG2JP.get(row["venue"], row["venue"])
        pay = payouts.get((row["date"], venue_jp, row["race_no"]))
        if pay is None:
            skipped["払戻が無い"] += 1
            continue
        legs = (row["slip"] or {}).get("legs") or []
        if not legs:
            skipped["伝票が空"] += 1
            continue
        stake, ret, hit_legs, by_type = settle(legs, pay)
        if stake <= 0:
            skipped["賭金ゼロ"] += 1
            continue
        races.append({
            "race_id": rid, "date": row["date"], "venue": venue_jp, "race_no": row["race_no"],
            "judged_roi": row["roi"], "stake": stake, "ret": ret,
            "realized_roi": ret / stake * 100.0, "hit_legs": hit_legs, "by_type": by_type,
            "konsen": row["konsen"], "race_budget": (row["slip"] or {}).get("race_budget"),
            "market_fair": market_fair_roi(legs, nearest_odds(odds.get(rid, {}),
                                                             row["captured_at"])),
            # 配分の入れ替え（#600）用。脚と払戻をそのまま持ち、金額だけ再計算できるようにする。
            "legs": legs, "payouts": pay,
            "has_box": any(leg.get("method") == "box" for leg in legs),
        })
    return races, dict(skipped)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--payouts-dir", required=True, help="payouts_YYYYMMDD.json を置いたディレクトリ")
    ap.add_argument("--db-url", default=os.environ.get(
        "PADDOCK_DB_URL", "postgres://paddock:paddock@127.0.0.1:5432/paddock"))
    ap.add_argument("--from", dest="date_from", default="2026-01-01")
    ap.add_argument("--to", dest="date_to", default="2026-12-31")
    ap.add_argument("--buckets", default="20,40,60,80", help="較正バケットの境界（%%・カンマ区切り）")
    ap.add_argument("--ever", action="store_true",
                    help="発走前最終スイープではなく全スイープ中の最大 ROI で評価する")
    ap.add_argument("--dump-races", help="レース単位の評価を TSV で書き出す")
    ap.add_argument("--compare-alloc", action="append", default=[], metavar="馬連,ワイド,3連複",
                    help="券種配分を入れ替えたときの実現ROIを比較する（脚は記録どおり・金額だけ再計算）。"
                         "複数回指定可。例: --compare-alloc 1500,1500,2000 --compare-alloc 1,1,1")
    args = ap.parse_args()

    payouts = load_payouts_dir(args.payouts_dir)
    by_race = load_live_ev(psql_dump_live_ev(args.db_url, args.date_from, args.date_to))
    odds = load_odds(psql_dump_odds(args.db_url, args.date_from, args.date_to))
    races, skipped = build_races(by_race, payouts, odds, use_ever=args.ever)

    print(f"live_ev_snapshots: {len(by_race)} レース / 評価できた {len(races)} レース"
          f"（{'ever' if args.ever else 'final'} 基準）")
    for reason, cnt in sorted(skipped.items()):
        print(f"  除外 {cnt}: {reason}")
    if not races:
        print("評価対象がゼロ。払戻の取得範囲と期間指定を確認する。", file=sys.stderr)
        return 1

    report(races, [float(x) for x in args.buckets.split(",") if x.strip()])
    if args.compare_alloc:
        report_alloc_comparison(races, [parse_alloc(a) for a in args.compare_alloc])
    report_axis_stability(by_race)

    if args.dump_races:
        with open(args.dump_races, "w") as f:
            f.write("race_id\tdate\tvenue\trace_no\tjudged_roi\tstake\tret\trealized_roi\t"
                    "market_fair\tkonsen\n")
            for r in races:
                mf = "" if r["market_fair"] is None else f"{r['market_fair']:.4f}"
                f.write(f"{r['race_id']}\t{r['date']}\t{r['venue']}\t{r['race_no']}\t"
                        f"{r['judged_roi']:.4f}\t{r['stake']:.0f}\t{r['ret']:.0f}\t"
                        f"{r['realized_roi']:.4f}\t{mf}\t"
                        f"{'t' if r['konsen'] else 'f'}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
