"""ライブ EV（期待回収率）計算 — 当日・発走前の最新オッズでレースを期待値評価する.

「高的中・低配当」を避け、**全3券種ROI（期待回収率）が +EV（≥100%）のレースだけ張る**ための
評価器。買い方は確定方針（¥5,000・ワイド/馬連/3連複を model 確率で重み付け配分・混戦は
3連複ボックス追加。strategy_eval.py と同系）と同一。的中確率は Plackett-Luce で
model 勝率→着順確率に変換し、実オッズを掛けて ROI を出す。

入力はすべて中間 TSV/テキスト（`refresh_ev.sh` が生成。konsen_backtest と同じ流儀）:
  --pred     predict 確率テーブル（`--- レース N: 場 馬場 距離m ---` + `馬番 名前 勝率% ...`）
  --horses   `pid<TAB>馬番<TAB>馬名<TAB>騎手<TAB>人気<TAB>単勝` （DB race_odds.win）
  --exotic   `pid<TAB>quinella|trio<TAB>組(a-b[-c])<TAB>オッズ` （DB race_odds）
  --wide     `pid<TAB>a-b<TAB>mid` （fetch_wide.py / netkeiba type=5）

ROI = Σ(賭金 × 的中確率 × 実配当) / Σ賭金。--slip で +EV レースの買い目伝票も出す。
"""
import argparse
import re
from itertools import combinations, permutations
from pathlib import Path

CJ = "①②③④⑤⑥⑦⑧⑨⑩⑪⑫⑬⑭⑮⑯⑰⑱"


def c(n):
    return CJ[n - 1] if 1 <= n <= 18 else f"[{n}]"


# --- 買い方（[[feedback-betting-staking]] と同一ロジック） ---
def band_of(probs):
    """◎の model 勝率の 0.70 倍以上の馬（◎含む）を勝率降順で返す。混戦判定の母集団。"""
    ax = max(probs, key=lambda n: probs[n])
    return sorted([n for n in probs if probs[n] >= 0.70 * probs[ax]], key=lambda n: -probs[n])


def is_konsen(probs):
    return len(band_of(probs)) >= 4


def largest_remainder(weights, units, minu=1):
    """weights 比で units を整数配分（各 minu 以上）。100円単位の最大剰余法。"""
    n = len(weights)
    if n == 0:
        return []
    s = sum(weights)
    if s <= 0:
        weights = [1] * n
        s = n
    rem = units - minu * n
    if rem < 0:  # 全点に minu を置けない → 重い順に1ずつ（units 点だけ）
        order = sorted(range(n), key=lambda i: weights[i], reverse=True)
        out = [0] * n
        for i in range(min(units, n)):  # minu>1 等で units>n になっても添字溢れしない防御
            out[order[i]] = 1
        return out
    ideal = [rem * w / s for w in weights]
    fl = [int(x) for x in ideal]
    al = [minu + fl[i] for i in range(n)]
    left = rem - sum(fl)
    frac = sorted(range(n), key=lambda i: ideal[i] - fl[i], reverse=True)
    for i in range(left):
        al[frac[i % n]] += 1
    return al


# --- Plackett-Luce 的中確率 ---
def p_top2_set(probs, a, b):
    """a,b がともに1-2着（馬連的中）になる確率。"""
    z = sum(probs.values())
    pa, pb = probs[a], probs[b]
    r = 0.0
    if z - pa > 0:  # 1頭が場の全確率を占める等で分母0になるのを防ぐ
        r += pa / z * pb / (z - pa)
    if z - pb > 0:
        r += pb / z * pa / (z - pb)
    return r


def p_top3_set(probs, trio):
    """trio の3頭がちょうど上位3着を占める確率（3連複的中）。"""
    z = sum(probs.values())
    s = 0.0
    for x, y, w in permutations(trio):
        d1 = z - probs[x]
        d2 = z - probs[x] - probs[y]
        if d1 <= 0 or d2 <= 0:  # 上位2頭で場の全確率に達する等の縮退で分母0を防ぐ
            continue
        s += probs[x] / z * probs[y] / d1 * probs[w] / d2
    return s


def p_pair_top3(probs, a, b):
    """a,b がともに3着以内（ワイド的中）。第3頭を全探索して和を取る。"""
    return sum(p_top3_set(probs, (a, b, third)) for third in probs if third not in (a, b))


# --- 入力パース ---
def parse_pred(path):
    text = Path(path).read_text()
    blocks = re.split(r"--- レース (\d+): (\S+) (\S+) (\d+)m ---", text)
    out = {}
    i = 1
    while i + 4 < len(blocks):
        rnum, ven, surf, dist, body = (int(blocks[i]), blocks[i + 1], blocks[i + 2],
                                       blocks[i + 3], blocks[i + 4])
        probs = {}
        for line in body.splitlines():
            m = re.match(r"\s*(\d+)\s+\S+\s+([\d.]+)%", line)
            if m:
                probs[int(m.group(1))] = float(m.group(2))
        if probs:
            out[(ven, rnum)] = dict(probs=probs, surface=surf, dist=int(dist))
        i += 5
    return out


def parse_meta(path):
    rows = []
    for line in Path(path).read_text().splitlines():
        if line.strip():
            cc = line.split("\t")
            rows.append(dict(pid=cc[0], venue=cc[1], rnum=int(cc[2])))
    return rows


def parse_horses(path):
    d = {}
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        pid, uma, name, jk, pop, odds = line.split("\t")
        d.setdefault(pid, {})[int(uma)] = dict(
            name=name, odds=float(odds) if odds else 0.0)
    return d


def parse_wide(path):
    d = {}
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        pid, pair, o = line.split("\t")
        a, b = sorted(int(x) for x in pair.split("-"))
        d.setdefault(pid, {})[(a, b)] = float(o)
    return d


def parse_exotic(path):
    qn, tr = {}, {}
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        pid, kind, combo, o = line.split("\t")
        nums = tuple(sorted(int(x) for x in combo.split("-")))
        try:
            ov = float(o)
        except ValueError:
            continue
        if kind == "quinella":
            qn.setdefault(pid, {})[nums] = ov
        elif kind == "trio":
            tr.setdefault(pid, {})[nums] = ov
    return qn, tr


def alloc(konsen):
    """券種別予算配分（¥5,000基準の比）。(box, wide, quinella, trio)。"""
    if konsen:
        return 1500, 1000, 1000, 1500
    return 0, 1500, 1500, 2000


def build_bets(probs, budget):
    """買い目を組む。返り値: list[(kind, combo_tuple, stake)]（kind=wide/quinella/trio）。

    予算は100円単位（端数は切り捨て）。総ユニット(budget//100)を券種レイヤーへ alloc 比で
    最大剰余配分し、レイヤー合計が必ず総ユニットに一致するようにする。これで任意の budget
    （5,000 の倍数でなくても）で総賭金が `budget//100*100` ちょうどになり、予算を取りこぼさない。
    """
    ax = max(probs, key=lambda n: probs[n])
    ranked = sorted(probs, key=lambda n: -probs[n])
    parts = [n for n in ranked if n != ax][:5]
    kon = is_konsen(probs)
    total_units = budget // 100
    # alloc 比でレイヤー（box/wide/馬連/3連複）へユニットを厳密配分（合計 = total_units）。
    u_box, u_wide, u_ren, u_trio = largest_remainder(list(alloc(kon)), total_units, minu=0)
    bets = []
    if kon and u_box > 0:
        # 混戦時は「印馬ボックス」と後段の「◎軸ながし」を別レイヤーとして重ねる設計
        # （◎が飛んでも印が揃えば拾うボックス保険 + ◎軸の本線）。両者で同一組番が
        # 出ることはあり、その場合は実際にその組へ二重に賭ける意図（金額は加算）。
        box = band_of(probs)[:5]
        combos = list(combinations(box, 3))
        for combo, u in zip(combos, largest_remainder(
                [probs[a] * probs[b] * probs[d] for a, b, d in combos], u_box)):
            if u > 0:
                bets.append(("trio", tuple(sorted(combo)), u * 100))
    wp = parts[:3]
    for n, u in zip(wp, largest_remainder([probs[n] for n in wp], u_wide)):
        if u > 0:
            bets.append(("wide", tuple(sorted((ax, n))), u * 100))
    for n, u in zip(parts, largest_remainder([probs[n] for n in parts], u_ren)):
        if u > 0:
            bets.append(("quinella", tuple(sorted((ax, n))), u * 100))
    pairs = list(combinations(parts, 2))
    for (a, b), u in zip(pairs, largest_remainder(
            [probs[a] * probs[b] for a, b in pairs], u_trio)):
        if u > 0:
            bets.append(("trio", tuple(sorted((ax, a, b))), u * 100))
    return ax, parts, kon, bets


def race_roi(probs, bets, wide, qn, tr):
    """買い目の期待回収率（ROI[%]）・総賭金・オッズ欠落の賭金合計を返す。

    オッズが取得できなかった買い目は分子（期待配当）に寄与できないが、分母（stake）には
    そのまま残す。これは保守的な選択で、欠落分を分母から外すと ROI を楽観方向に水増しして
    +EV を誤検出するため。欠落額（missing）を返し、呼び出し側で「ROI の信頼度が低い」旨を
    可視化する。
    """
    books = {"wide": wide, "quinella": qn, "trio": tr}
    probfn = {
        "wide": lambda cb: p_pair_top3(probs, *cb),
        "quinella": lambda cb: p_top2_set(probs, *cb),
        "trio": lambda cb: p_top3_set(probs, cb),
    }
    ret = 0.0
    stake = 0
    missing = 0
    for kind, combo, amt in bets:
        stake += amt
        o = books[kind].get(combo)
        if o:
            ret += amt * probfn[kind](combo) * o
        else:
            missing += amt
    return (ret / stake * 100 if stake else 0.0), stake, missing


def print_slip(venue, rnum, ax, h, probs, bets):
    name = h.get(ax, {}).get("name", "?")
    print(f"\n  === {venue}{rnum:02d}R 買い目 ◎{c(ax)} {name} "
          f"単勝{h.get(ax, {}).get('odds', '?')} model{probs[ax]:.1f}% ===")
    # 同一組番（混戦ボックスと◎軸ながしで重複しうる）は購入額を合算して1行で表示する
    # （現場入力で同じ買い目が2行に割れて混乱しないように。賭金は合算）。
    by_kind = {"wide": {}, "quinella": {}, "trio": {}}
    for kind, combo, amt in bets:
        by_kind[kind][combo] = by_kind[kind].get(combo, 0) + amt
    label = {"wide": "ワイド", "quinella": "馬連", "trio": "3連複"}
    for kind in ("wide", "quinella", "trio"):
        items = by_kind[kind]
        if not items:
            continue
        print(f"  [{label[kind]}] 計¥{sum(items.values()):,}")
        for combo, amt in items.items():
            print(f"    {'-'.join(c(x) for x in combo)}  ¥{amt:,}")


def main():
    ap = argparse.ArgumentParser(description="ライブ EV 計算（当日・発走前の最新オッズ）")
    ap.add_argument("--pred", required=True, help="predict 確率テーブル")
    ap.add_argument("--meta", required=True, help="pid/venue/rnum の TSV")
    ap.add_argument("--horses", required=True, help="馬番/馬名/単勝の TSV")
    ap.add_argument("--exotic", required=True, help="馬連/3連複オッズの TSV")
    ap.add_argument("--wide", required=True, help="ワイドオッズの TSV")
    ap.add_argument("--budget", type=int, default=5000, help="1レース予算（円, 既定5000）")
    ap.add_argument("--slip", action="store_true", help="+EV レースの買い目伝票も出力")
    args = ap.parse_args()

    preds = parse_pred(args.pred)
    metas = parse_meta(args.meta)
    horses = parse_horses(args.horses)
    wide = parse_wide(args.wide)
    qn, tr = parse_exotic(args.exotic)

    rows = []
    for m in metas:
        pinfo = preds.get((m["venue"], m["rnum"]))
        h = horses.get(m["pid"])
        if not pinfo or not h:
            continue
        probs = {n: v for n, v in pinfo["probs"].items() if n in h}
        if len(probs) < 3:
            continue
        ax, parts, kon, bets = build_bets(probs, args.budget)
        roi, _, missing = race_roi(probs, bets, wide.get(m["pid"], {}),
                                   qn.get(m["pid"], {}), tr.get(m["pid"], {}))
        rows.append((roi, m["venue"], m["rnum"], ax, h[ax]["odds"], probs[ax], kon,
                     h[ax]["name"], missing, h, probs, bets))

    rows.sort(key=lambda r: -r[0])
    print(f"=== EV ランキング（¥{args.budget:,}・全3券種ROI） ===")
    for roi, v, rn, ax, o, p, kon, name, missing, *_ in rows:
        flag = "✅+EV" if roi >= 100 else " −EV"
        km = "[混戦]" if kon else "    "
        warn = "  ⚠オッズ欠落" if missing else ""
        print(f"  {flag} ROI{roi:5.0f}%  {v}{rn:02d}R {km} ◎{c(ax)} {name[:8]:<8} "
              f"単勝{o:>5} model{p:4.1f}%{warn}")
    pos = [r for r in rows if r[0] >= 100]
    print(f"\n+EV: {len(pos)} 本" + ("" if pos else "（現在ナシ）"))
    nmiss = sum(1 for r in rows if r[8] > 0)
    if nmiss:
        print(f"⚠ オッズ欠落のあるレース {nmiss} 本: ROI は過小評価（買い目の一部に配当が無い）。"
              f"fetch 失敗や wide_errors.log を確認のこと。")
    if args.slip:
        for roi, v, rn, ax, o, p, kon, name, missing, h, probs, bets in pos:
            print_slip(v, rn, ax, h, probs, bets)


if __name__ == "__main__":
    main()
