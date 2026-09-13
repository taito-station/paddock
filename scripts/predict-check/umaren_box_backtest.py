#!/usr/bin/env python3
"""印馬の馬連ボックスを配分に足す案のバックテスト（#629）。

現行の買い目は全券種が ◎1頭軸ながし（wide/umaren/sanrenpuku）で、◎が3着以内に来ないと
全損する。印馬（◎+相手top5=6頭）の馬連ボックスを既存券種のどれかと置き換えれば、◎が飛んでも
相手同士の的中（1-2着が印内）で拾える、という仮説を過去データで検証する。

検証は 2 段構成:
  (A) 頻度  : ◎が飛んだレースのうち、1-2着が印6頭/5頭内に収まる割合
              （box が原理的に拾えるチャンスの上限）
  (B) ROI   : baseline（現行ルール）と、wide/umaren/sanrenpuku のいずれかを
              馬連ボックス（6頭 or 5頭）に置換した variant を全レース機械適用で比較
  (C)       : (B) を混戦/非混戦で分割（box variant は非混戦時のみ追加。混戦時は
              既に3連複ボックスが入るため box variant を上乗せせず baseline にフォールバック）
  (D)       : 5頭 vs 6頭 は (B) の box5/box6 行を比較すればよい（別集計は無い）

回収率の分母は「実際に賭けた額」（#180/#250 と同方針）。

入力（konsen_backtest.py と同じ TSV 形式・regex を独立コピー。konsen_backtest.py 側の
内部実装変更に追従して意図せず結果が変わることを避けるため import しない）:
  --races        TSV: date, paddock_id, venue_jp, round, day, race_num, netkeiba_id
  --winodds      TSV: paddock_id, umaban, popularity, odds
  --pred-dir     dir: bt_pred_<date>.txt（model 単勝勝率表）
  --results-dir  dir: netkeiba result.html を res_<netkeiba>.html で保存したもの
  --exotic-odds  TSV: paddock_id, bet_type, combination_key, odds
                 （umaren_backtest.py と同形式。本スクリプトでは使わない——EV フィルタに使う
                   盤面オッズと的中時の払戻を同一オッズで循環させない方針を守るため、払戻は
                   常に result.html の実配当のみを使う。将来の拡張用に引数だけ受ける）

使い方:
  python3 umaren_box_backtest.py --races /tmp/bt_races.tsv --winodds /tmp/bt_winodds.tsv \
      --pred-dir /tmp --results-dir /tmp --exotic-odds /tmp/bt_exotic_odds.tsv
"""
import argparse
import re
import statistics
import sys
from itertools import combinations
from pathlib import Path

from pred_header import HEADER_NUM_VENUE, NoHeaderFound, split_by_header

BUDGET = 5000


# --- 入力パース（konsen_backtest.py と同一実装のコピー） ----------------------------
def parse_races(path):
    rows = []
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        c = line.split("\t")
        rows.append(dict(date=c[0], pid=c[1], venue=c[2], rnum=int(c[5]), nk=c[6]))
    return rows


def parse_winodds(path):
    d = {}
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        pid, uma, pop, odds = line.split("\t")
        d.setdefault(pid, {})[int(uma)] = (int(pop), float(odds))
    return d


def parse_pred(path):
    """predict 出力から (venue_jp, race_num) -> {umaban: win_prob} を抽出。"""
    text = Path(path).read_text()
    blocks = split_by_header(text, HEADER_NUM_VENUE, path)
    out = {}
    i = 1
    while i + 2 < len(blocks):
        rnum, venue, body = int(blocks[i]), blocks[i + 1], blocks[i + 2]
        probs = {}
        for line in body.splitlines():
            m = re.match(r"\s*(\d+)\s+\S+\s+([\d.]+)%", line)
            if m:
                probs[int(m.group(1))] = float(m.group(2))
        if probs:
            out[(venue, rnum)] = probs
        i += 3
    return out


def parse_result(path):
    """netkeiba result.html -> (top3 umaban list, payouts dict)。"""
    t = Path(path).read_text(encoding="utf-8")
    rows = re.split(r'<tr\b[^>]*class="[^"]*HorseList[^"]*"', t)[1:]
    order = []
    for r in rows:
        rk = re.search(r'class="Rank">(\d+)</div>', r)
        um = re.search(r'class="Num Txt_C">\s*<div>\s*(\d+)\s*</div>', r, re.S)
        if rk and um:
            order.append((int(rk.group(1)), int(um.group(1))))
    order.sort()
    top3 = [u for _, u in order[:3]]
    pay = {"umaren": {}, "wide": {}, "trio": {}}
    for key, cls in [("umaren", "Umaren"), ("wide", "Wide"), ("trio", "Fuku3")]:
        m = re.search(rf'<tr class="{cls}">(.*?)</tr>', t, re.S)
        if not m:
            continue
        combos = re.findall(r'class="Result">(.*?)</td>', m.group(1), re.S)
        pays = re.findall(r'class="Payout">(.*?)</td>', m.group(1), re.S)
        if not combos or not pays:
            continue
        nums = [int(x) for x in re.findall(r"\d+", re.sub(r"<[^>]+>", " ", combos[0]))]
        yens = [int(x.replace(",", "")) for x in re.findall(r"([\d,]+)円", re.sub(r"<[^>]+>", " ", pays[0]))]
        size = 3 if key == "trio" else 2
        if len(nums) != size * len(yens):
            continue
        for k in range(len(yens)):
            combo = frozenset(nums[k * size:(k + 1) * size])
            if len(combo) == size:
                pay[key][combo] = yens[k]
    return top3, pay


# --- 混戦判定（konsen_backtest.py の band_of/baseline 定義のコピー。odds しきい値の
#     拡張条件はこのスクリプトでは扱わない＝現行ルールの baseline 定義のみ再現） ------------
def band_of(probs):
    axis = max(probs, key=lambda n: probs[n])
    return sorted([n for n in probs if probs[n] >= 0.70 * probs[axis]], key=lambda n: -probs[n])


def is_konsen(probs):
    return len(band_of(probs)) >= 4


# --- 印馬・ボックス組合せ -----------------------------------------------------------
def get_marked_horses(probs, n_partners=5):
    """印馬 = ◎(axis) + 勝率降順の相手 top n_partners。返り値は [axis, 相手1, 相手2, ...]。

    先頭に axis を置く順序は settle_umaren_box の n_heads スライス（末尾から取る）が
    「6頭=axis+top5全部／5頭=相手top5のみ（axis を含まない）」を作り分ける前提になっている。
    """
    axis = max(probs, key=lambda n: probs[n])
    partners = sorted([n for n in probs if n != axis], key=lambda n: -probs[n])[:n_partners]
    return [axis] + partners


def box_pairs(heads):
    """heads から C(n,2) の全ペアを frozenset のリストで返す（馬連ボックスの組）。"""
    return [frozenset(p) for p in combinations(heads, 2)]


def uniform_alloc(budget_yen, n_points):
    """budget_yen を n_points 点に 100円単位で均等配分。単位は「100円の口数」の list[int]。

    全点に最低 1 口（¥100）配れる予算があれば 1 口ずつ配り、余りは先頭の点から 1 口ずつ積む。
    配れない予算（口数 < 点数）は先頭の点から順に 1 口ずつ埋め、残りは ¥0（賭けない）。
    """
    if n_points <= 0:
        return []
    total_units = budget_yen // 100
    if total_units >= n_points:
        base, rem = divmod(total_units, n_points)
        alloc = [base] * n_points
        for i in range(rem):
            alloc[i] += 1
        return alloc
    alloc = [0] * n_points
    for i in range(total_units):
        alloc[i] = 1
    return alloc


def is_box_opportunity(axis, marked, first, second, top3):
    """(A) 頻度判定: ◎が top3 外 かつ 1-2着が両方 marked 内。

    marked は 6頭（axis+top5）でも 5頭（top5 のみ）でも呼び出し側が渡した集合をそのまま使う。
    """
    return axis not in top3 and first in marked and second in marked


# --- settle: baseline（現行ルール） -------------------------------------------------
def _leg_wide(axis, parts, pay, budget_yen):
    ret = stake = 0
    alloc = uniform_alloc(budget_yen, len(parts))
    for n, u in zip(parts, alloc):
        stake += u * 100
        ret += u * 100 * pay["wide"].get(frozenset({axis, n}), 0) // 100
    return ret, stake


def _leg_umaren(axis, parts, pay, budget_yen):
    ret = stake = 0
    alloc = uniform_alloc(budget_yen, len(parts))
    for n, u in zip(parts, alloc):
        stake += u * 100
        ret += u * 100 * pay["umaren"].get(frozenset({axis, n}), 0) // 100
    return ret, stake


def _leg_sanrenpuku(axis, parts, pay, budget_yen):
    pairs = list(combinations(parts, 2))
    ret = stake = 0
    alloc = uniform_alloc(budget_yen, len(pairs))
    for (a, b), u in zip(pairs, alloc):
        stake += u * 100
        ret += u * 100 * pay["trio"].get(frozenset({axis, a, b}), 0) // 100
    return ret, stake


def _leg_trio_box(probs, pay, budget_yen):
    """混戦時の3連複ボックス（band 最大5頭）。"""
    box = band_of(probs)[:5]
    combos = list(combinations(box, 3))
    ret = stake = 0
    alloc = uniform_alloc(budget_yen, len(combos))
    for (a, b, c), u in zip(combos, alloc):
        stake += u * 100
        ret += u * 100 * pay["trio"].get(frozenset({a, b, c}), 0) // 100
    return ret, stake


def settle_baseline(probs, top3, pay, konsen):
    """現行ルール（konsen_backtest.py settle_race と同一構造）で1レースを清算。

    konsen=True: 3連複ボックス¥1500 追加 + wide/umaren ¥1000 / sanrenpuku ¥1500
    konsen=False: wide/umaren ¥1500 / sanrenpuku ¥2000（全て ◎軸ながし）
    top3 は現行実装では未使用（settle_race 同様、呼び出しインタフェースの対称性のために残す）。
    """
    axis = max(probs, key=lambda n: probs[n])
    parts = sorted([n for n in probs if n != axis], key=lambda n: -probs[n])[:5]
    ret = stake = 0
    if konsen:
        r, s = _leg_trio_box(probs, pay, 1500)
        ret += r
        stake += s
        bw, bm, bf = 1000, 1000, 1500
    else:
        bw, bm, bf = 1500, 1500, 2000
    r, s = _leg_wide(axis, parts, pay, bw)
    ret += r
    stake += s
    r, s = _leg_umaren(axis, parts, pay, bm)
    ret += r
    stake += s
    r, s = _leg_sanrenpuku(axis, parts, pay, bf)
    ret += r
    stake += s
    return ret, stake


# --- settle: 馬連ボックス variant ---------------------------------------------------
def settle_umaren_box(marked, pay_umaren, budget_yen, n_heads):
    """marked の末尾 n_heads 頭で馬連ボックスを組み、budget_yen を均等配分して清算。

    marked = [axis, 相手1, ..., 相手5]（get_marked_horses の並び）なので、
    末尾から n_heads 取ると n_heads=6 で axis+相手5頭全部、n_heads=5 で相手5頭のみ（axis 抜き）になる。
    """
    heads = marked[-n_heads:] if n_heads < len(marked) else list(marked)
    pairs = box_pairs(heads)
    alloc = uniform_alloc(budget_yen, len(pairs))
    ret = stake = 0
    for combo, u in zip(pairs, alloc):
        amt = u * 100
        stake += amt
        ret += amt * pay_umaren.get(combo, 0) // 100
    return ret, stake


def settle_variant(probs, marked, top3, pay, konsen, replace_leg, n_heads):
    """baseline のどれか1券種（wide/umaren/sanrenpuku）を馬連ボックスに置換して清算。

    混戦時は box variant を上乗せしない（既に3連複ボックスが入るため）。baseline と同一の
    挙動にフォールバックする——同じレース集合で比較するため、混戦レースを box variant から
    除外するのではなく baseline 値をそのまま返す。
    """
    if konsen:
        return settle_baseline(probs, top3, pay, konsen=True)

    axis = max(probs, key=lambda n: probs[n])
    parts = sorted([n for n in probs if n != axis], key=lambda n: -probs[n])[:5]
    bw, bm, bf = 1500, 1500, 2000
    pay_umaren = pay["umaren"]
    ret = stake = 0

    if replace_leg == "wide":
        r, s = settle_umaren_box(marked, pay_umaren, bw, n_heads)
    else:
        r, s = _leg_wide(axis, parts, pay, bw)
    ret += r
    stake += s

    if replace_leg == "umaren":
        r, s = settle_umaren_box(marked, pay_umaren, bm, n_heads)
    else:
        r, s = _leg_umaren(axis, parts, pay, bm)
    ret += r
    stake += s

    if replace_leg == "sanrenpuku":
        r, s = settle_umaren_box(marked, pay_umaren, bf, n_heads)
    else:
        r, s = _leg_sanrenpuku(axis, parts, pay, bf)
    ret += r
    stake += s

    return ret, stake


# --- 集計表示 ------------------------------------------------------------------------
BOX_VARIANTS = [
    ("wide→box6", "wide", 6),
    ("sanrenpuku→box6", "sanrenpuku", 6),
    ("umaren→box6", "umaren", 6),
    ("wide→box5", "wide", 5),
    ("sanrenpuku→box5", "sanrenpuku", 5),
    ("umaren→box5", "umaren", 5),
]


def _summarize(rows):
    """rows = [(ret, stake)] -> (回収率%, 的中率%, σ(per-race ROI%), 損益円)。"""
    if not rows:
        return 0.0, 0.0, 0.0, 0
    tot_ret = sum(r for r, _ in rows)
    tot_stake = sum(s for _, s in rows)
    roi = tot_ret / tot_stake * 100 if tot_stake else 0.0
    hit = sum(1 for r, _ in rows if r > 0) / len(rows) * 100
    per = [r / s * 100 if s else 0.0 for r, s in rows]
    sd = statistics.stdev(per) if len(per) > 1 else 0.0
    pnl = tot_ret - tot_stake
    return roi, hit, sd, pnl


def _print_roi_table(rows_by_variant, variant_order):
    print(f"{'variant':<18} {'回収率':>8} {'的中率':>8} {'σ(ROI)':>8} {'損益':>10}")
    for name in variant_order:
        roi, hit, sd, pnl = _summarize(rows_by_variant[name])
        print(f"{name:<18} {roi:>7.1f}% {hit:>7.1f}% {sd:>8.1f} {pnl:>+10}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--races", default="/tmp/bt_races.tsv")
    ap.add_argument("--winodds", default="/tmp/bt_winodds.tsv")
    ap.add_argument("--pred-dir", default="/tmp")
    ap.add_argument("--results-dir", default="/tmp")
    ap.add_argument("--exotic-odds", default=None,
                     help="#629 の循環回避方針により未使用（払戻は result.html の実配当のみ使う）。"
                          "将来の拡張用に引数だけ受ける")
    args = ap.parse_args()

    if args.exotic_odds:
        print("注: --exotic-odds は循環回避方針により未使用。払戻は result.html の実配当のみ使用\n",
              file=sys.stderr)

    races = parse_races(args.races)
    winodds = parse_winodds(args.winodds)
    preds = {}
    for d in sorted({r["date"] for r in races}):
        p = Path(args.pred_dir) / f"bt_pred_{d}.txt"
        if p.exists():
            preds[d] = parse_pred(p)

    variant_order = ["baseline"] + [name for name, *_ in BOX_VARIANTS]
    rows_all = {name: [] for name in variant_order}
    rows_konsen = {name: [] for name in variant_order}
    rows_nonkonsen = {name: [] for name in variant_order}

    total = axis_top3 = axis_off = hit6 = 0
    n_konsen = n_nonkonsen = 0

    skips = dict(probs=0, odds=0, result=0)
    for r in races:
        probs = preds.get(r["date"], {}).get((r["venue"], r["rnum"]))
        wo = winodds.get(r["pid"])
        resf = Path(args.results_dir) / f"res_{r['nk']}.html"
        if not probs:
            skips["probs"] += 1
            continue
        if not wo:
            skips["odds"] += 1
            continue
        if not resf.exists():
            skips["result"] += 1
            continue
        top3, pay = parse_result(resf)
        if len(top3) < 3:
            skips["result"] += 1
            continue

        axis = max(probs, key=lambda n: probs[n])
        marked = get_marked_horses(probs, n_partners=5)
        konsen = is_konsen(probs)

        total += 1
        if konsen:
            n_konsen += 1
        else:
            n_nonkonsen += 1
        first, second = top3[0], top3[1]
        if axis in top3:
            axis_top3 += 1
        else:
            axis_off += 1
            if is_box_opportunity(axis, marked, first, second, top3):
                hit6 += 1

        ret, stake = settle_baseline(probs, top3, pay, konsen)
        rows_all["baseline"].append((ret, stake))
        (rows_konsen if konsen else rows_nonkonsen)["baseline"].append((ret, stake))

        for name, replace_leg, n_heads in BOX_VARIANTS:
            ret, stake = settle_variant(probs, marked, top3, pay, konsen, replace_leg, n_heads)
            rows_all[name].append((ret, stake))
            (rows_konsen if konsen else rows_nonkonsen)[name].append((ret, stake))

    skipped = sum(skips.values())
    print(
        f"対象レース: {total}（スキップ {skipped}: probs欠落 {skips['probs']} / "
        f"odds欠落 {skips['odds']} / result欠落 {skips['result']}）\n"
    )

    print("=== (A) 軸飛び＋印内的中の頻度 ===")
    print(f"全レース: {total}")
    top3_pct = axis_top3 / total * 100 if total else 0.0
    off_pct = axis_off / total * 100 if total else 0.0
    print(f"  ◎ top3 入り: {axis_top3} ({top3_pct:.1f}%)")
    print(f"  ◎ 飛び: {axis_off} ({off_pct:.1f}%)")
    hit6_pct = hit6 / axis_off * 100 if axis_off else 0.0
    print(f"    うち 1-2着が印6頭内: {hit6} (◎飛びの {hit6_pct:.1f}%)")
    print()

    print("=== (B) ROI 比較（全レース） ===")
    _print_roi_table(rows_all, variant_order)
    print()

    print("=== (C) 混戦/非混戦別 ===")
    print(f"[非混戦: {n_nonkonsen} レース]")
    _print_roi_table(rows_nonkonsen, variant_order)
    print()
    print(f"[混戦: {n_konsen} レース]")
    _print_roi_table(rows_konsen, variant_order)


if __name__ == "__main__":
    # 見出しが取れない異常は案内文だけを出して非 0 終了する（#587）。traceback だと
    # せっかく 1 本化した案内が埋もれる。
    try:
        main()
    except NoHeaderFound as e:
        sys.exit(str(e))
