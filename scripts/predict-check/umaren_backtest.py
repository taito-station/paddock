#!/usr/bin/env python3
"""馬連・馬単特化 買い目戦略のバックテスト（#250 / #262）。

「ROI≥100% で張れる鞍が極端に少ない」運用課題に対し、+EV が集中する馬連に券種を絞ると
−EV 閾値を下げずに「張れる鞍数（frequency）」を増やせるかを過去データで検証する（#250）。
あわせて馬単（exacta）特化を同一基盤で検証し、順序プレミアム（着順固定配当 > 着順不問配当を
モデルが正しく EV 化できているか）が実エッジか・EV フィルタが逆予測でないかを判定する（#262）。
馬単は馬連と対称に ◎軸ながしマルチ（各相手につき (◎>相手)・(相手>◎) の順序 2 レッグ）を universe とし、
exacta model EV ≥ θ の点のみ抽出する。同一窓・同一モデルで馬連と並べて比較する。

対照（baseline_pf）と比較する:
  - baseline_pf : 現行ルール−wide（馬連 ◎軸ながし相手 top5 ¥1500 + 三連複 ◎軸ながし相手 top5 ¥2000、
                  確率重み配分）。発動ゲート = ポートフォリオ model ROI ≥ 100%。
                  ※ wide は履歴オッズ欠落のため除外。課題の主張「wide が EV を希薄化」に照らせば
                    wide 抜き baseline はむしろ強い対照（steel-man）になる。
  - umaren_only : ◎1頭軸ながし（軸=◎の 1 頭固定, #241 準拠）。相手は top5 に限らず**全頭**から
                  馬連 model EV ≥ θ の点のみ抽出する（+EV は穴に偏るため top5 では捕れない）。
                  予算 ¥5,000 を選抜点に配分（flat=均等 / weighted=馬連的中確率重み）。
                  発動ゲート = +EV 点が 1 点以上。θ は --ev-grid・オッズ上限 cap は --cap-grid で掃引。

対照（参考。いずれも全鞍機械買い）:
  - 馬連top5 無フィルタ      : ◎軸ながし・相手 top5・¥1,500（現行ルールの馬連分・confound 用の堅め対照）。
  - 馬連 全頭ながし 無フィルタ: ◎軸ながし・相手全頭・¥5,000・flat。umaren_only と universe（全頭）・配分
                  （flat）を揃え EV フィルタの有無だけを変えた apples-to-apples 対照。これより
                  umaren_only が悪ければ「model EV ランキングが逆予測的」と切り分けられる。

【循環回避】EV フィルタに使うオッズ（DB 盤面、bt_exotic_odds.tsv）と、的中時の払戻
（result.html の実配当）を分離する。同一オッズで清算すると「EV≥θ の点は当たれば必ず θ 倍返る」
恒真化で umaren_only が不当有利になるため。これによりモデル確率の較正不良（穴の過大評価, #246）が
実 ROI に正しく反映される。

指標（発動部分集合 = 実際に賭けた鞍のみ）:
  frequency（張れた鞍数）/ ROI / 的中率 / σ(per-race ROI) / 最大ドローダウン（累積損益, 円）。
回収率の分母は「実際に賭けた額」（#180/#241 と同方針）。

入力（既定 /tmp/bt250、gen_win_backtest_data.sh が生成。--exotic-odds のみ別途 DB エクスポート）:
  --races        TSV: date, paddock_id, venue_jp, round, day, race_num, netkeiba_id
  --pred-dir     dir: bt_pred_<date>.txt（model 単勝勝率表、analyze predict --blend-alpha 0.2）
  --results-dir  dir: netkeiba result.html を res_<netkeiba>.html で保存したもの
  --exotic-odds  TSV: paddock_id, bet_type(quinella|trio|exacta), combination_key, odds
                  （exacta の combination_key は順序付き '1着>2着'、無順券種は '-' 連結）
  --winodds      TSV: paddock_id, umaban, popularity, odds（#263 ゲート診断の市場人気度。欠落可）

#263 の baseline_pf ゲート精度診断/掃引:
  --gate-grid        baseline_pf ゲート閾値（model ROI）の掃引。0=無ゲート対照（既定 0,1.0,1.1,1.2,1.3）
  --odds-floor-grid  ◎単勝オッズ下限の掃引。0=条件なし（既定 0,2,3,5）
  ※ #246 較正後の検証は #252 と同じ /tmp/bt252 入力（較正後再生成）を明示指定する。

bt_exotic_odds.tsv の生成（DB から、71R 窓の馬連・三連複・馬単盤面）:
  psql "$DB" -tA -F$'\t' -c "
    SELECT o.race_id, o.bet_type, o.combination_key, o.odds::text
    FROM race_odds o JOIN race_cards rc ON rc.race_id=o.race_id
    WHERE o.bet_type IN ('quinella','trio','exacta')
      AND rc.date >= '2026-05-30' AND rc.date <= '2026-06-14'
    ORDER BY o.race_id, o.bet_type, o.odds;" > /tmp/bt250/bt_exotic_odds.tsv

使い方:
  python3 umaren_backtest.py --races /tmp/bt250/bt_races.tsv --pred-dir /tmp/bt250 \
      --results-dir /tmp/bt250 --exotic-odds /tmp/bt250/bt_exotic_odds.tsv \
      --ev-grid 1.0,1.2,1.5 --cap-grid inf,50,30
"""
import argparse
import re
import statistics
import unicodedata
from itertools import combinations, permutations
from pathlib import Path


def largest_remainder(weights, units, minu=1):
    """重み比で units 口を整数配分（各目に最低 minu 口）。formation/konsen_backtest と同一実装。"""
    n = len(weights)
    if n == 0:
        return []
    s = sum(weights)
    if s <= 0:
        weights = [1] * n
        s = n
    base = [minu] * n
    rem = units - minu * n
    if rem < 0:
        order = sorted(range(n), key=lambda i: weights[i], reverse=True)
        out = [0] * n
        for i in range(units):
            out[order[i]] = 1
        return out
    ideal = [rem * w / s for w in weights]
    fl = [int(x) for x in ideal]
    alloc = [base[i] + fl[i] for i in range(n)]
    left = rem - sum(fl)
    order = sorted(range(n), key=lambda i: ideal[i] - fl[i], reverse=True)
    for i in range(left):
        alloc[order[i % n]] += 1
    return alloc


# --- Plackett-Luce 的中確率（live_ev.py より移植） ---
def p_top2_set(probs, a, b):
    """a,b がともに 1-2 着（馬連的中）になる確率。"""
    z = sum(probs.values())
    pa, pb = probs[a], probs[b]
    r = 0.0
    if z - pa > 0:
        r += pa / z * pb / (z - pa)
    if z - pb > 0:
        r += pb / z * pa / (z - pb)
    return r


def p_exacta(probs, a, b):
    """a=1着・b=2着 の順序付き的中確率（馬単的中）。Plackett-Luce の単一順序。

    p_top2_set（無順・馬連）と対称: p_exacta(a,b) + p_exacta(b,a) == p_top2_set(a,b)。
    """
    z = sum(probs.values())
    pa = probs[a]
    if z <= 0 or z - pa <= 0:
        return 0.0
    return pa / z * probs[b] / (z - pa)


def p_top3_set(probs, trio):
    """trio の 3 頭がちょうど上位 3 着を占める確率（三連複的中）。"""
    z = sum(probs.values())
    s = 0.0
    for x, y, w in permutations(trio):
        d1 = z - probs[x]
        d2 = z - probs[x] - probs[y]
        if d1 <= 0 or d2 <= 0:
            continue
        s += probs[x] / z * probs[y] / d1 * probs[w] / d2
    return s


# --- 入力パース（formation_backtest.py と同一） ---
def parse_races(path):
    rows = []
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        c = line.split("\t")
        rows.append(dict(date=c[0], pid=c[1], venue=c[2], rnum=int(c[5]), nk=c[6]))
    return rows


def parse_pred(path):
    """predict 出力から (venue_jp, race_num) -> {umaban: win_prob(%)} を抽出。"""
    text = Path(path).read_text()
    blocks = re.split(r"--- レース (\d+): (\S+) \S+ \d+m ---", text)
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
    """netkeiba result.html -> (top3 umaban list, payouts dict)。formation_backtest と同一。"""
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
    pay = {"umaren": {}, "wide": {}, "trio": {}, "exacta": {}}
    # exacta（馬単・Umatan）は着順保持のため出現順タプルでキー化。他は無順 frozenset。
    # netkeiba result.html の Umatan 組は li が「1着→2着」順で並ぶ（実 result で実証済み・
    # src/interface/netkeiba-scraper/src/parse/payout.rs「順序付きは出現順 > 連結」と一致）。
    # 同着1着では複数の馬単組が並ぶため出現順をそのまま保持する（着順固定の hard assert は不可）。
    for key, cls in [("umaren", "Umaren"), ("wide", "Wide"), ("trio", "Fuku3"), ("exacta", "Umatan")]:
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
            seg = nums[k * size:(k + 1) * size]
            combo = tuple(seg) if key == "exacta" else frozenset(seg)
            if len(set(seg)) == size:
                pay[key][combo] = yens[k]
    return top3, pay


def parse_exotic(path):
    """bt_exotic_odds.tsv -> {pid: {"quinella": {frozenset: odds}, "trio": {frozenset: odds},
    "exacta": {(1着,2着): odds}}}。

    combination_key の形式は券種で異なる:
      - quinella/trio: 馬番ハイフン区切りの無順序（例 '9-15', '9-15-3'）→ frozenset。
      - exacta: 馬番 '>' 区切りの順序付き（例 '9>15' = 9 着順 1 着・15 着順 2 着）→ (a, b) タプル。
    """
    out = {}
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        pid, bt, key, odds = line.split("\t")
        if bt not in ("quinella", "trio", "exacta"):  # 想定外 bet_type を無視（TSV 手編集への防御）
            continue
        slot = out.setdefault(pid, {"quinella": {}, "trio": {}, "exacta": {}})
        if bt == "exacta":
            # exacta は '1着>2着' の順序付き 2 頭。区切り数が 2 でない異常行は無視する（DB は型付き export だが手編集 TSV への防御）。
            parts = key.split(">")
            if len(parts) != 2:
                continue
            slot[bt][(int(parts[0]), int(parts[1]))] = float(odds)
        else:
            slot[bt][frozenset(int(x) for x in key.split("-"))] = float(odds)
    return out


def parse_winodds(path):
    """bt_winodds.tsv -> {pid: {umaban: (popularity, odds)}}（#263 診断用の市場人気度）。

    TSV: paddock_id, umaban, popularity, odds。ファイルが無ければ空 dict（診断は odds 欠落扱い）。
    """
    out = {}
    p = Path(path)
    if not p.exists():
        return out
    for line in p.read_text().splitlines():
        if not line.strip():
            continue
        pid, um, pop, odds = line.split("\t")
        out.setdefault(pid, {})[int(um)] = (int(pop), float(odds))
    return out


# --- 戦略 ---------------------------------------------------------------------
def compute_baseline_pf(probs, quin_odds, trio_odds, pay):
    """現行ルール−wide ポートフォリオの (model_roi, ret, stake) を算出（ゲート判定はしない）。

    馬連 ◎軸ながし top5 ¥1500 + 三連複 ◎軸ながし top5 ¥2000（確率重み）。
    model_roi = 期待払戻 / 賭金（DB 盤面オッズで算出）。清算 ret は実払戻。
    DB オッズ欠落の目は model EV 寄与 0（保守的）。評価不能（3 頭未満）は (0.0, 0, 0)。

    ゲート閾値や ◎オッズ条件の掃引（#263）のため、計算と発動判定を分離している。
    """
    ranked = sorted(probs, key=lambda n: -probs[n])
    if len(ranked) < 3:
        return 0.0, 0, 0
    A = ranked[0]
    partners = ranked[1:6]

    um_combos = [frozenset({A, p}) for p in partners]
    um_stakes = [u * 100 for u in largest_remainder([probs[p] for p in partners], 1500 // 100)]

    tr_pairs = list(combinations(partners, 2))
    tr_combos = [frozenset({A, a, b}) for a, b in tr_pairs]
    tr_stakes = [u * 100 for u in largest_remainder([probs[a] * probs[b] for a, b in tr_pairs], 2000 // 100)]

    exp_ret = 0.0
    stake = 0
    ret = 0
    for p, c, s in zip(partners, um_combos, um_stakes):
        stake += s
        exp_ret += s * p_top2_set(probs, A, p) * quin_odds.get(c, 0.0)
        ret += s * pay["umaren"].get(c, 0) // 100
    for (a, b), c, s in zip(tr_pairs, tr_combos, tr_stakes):
        stake += s
        exp_ret += s * p_top3_set(probs, (A, a, b)) * trio_odds.get(c, 0.0)
        ret += s * pay["trio"].get(c, 0) // 100

    model_roi = exp_ret / stake if stake > 0 else 0.0
    return model_roi, ret, stake


def eval_baseline_pf(probs, quin_odds, trio_odds, pay, gate=1.0):
    """現行ルール−wide。発動ゲート = ポートフォリオ model ROI ≥ gate（既定 1.0=100%）。

    返り値 (bet, ret, stake)。計算は compute_baseline_pf に委譲。
    """
    model_roi, ret, stake = compute_baseline_pf(probs, quin_odds, trio_odds, pay)
    bet = stake > 0 and model_roi >= gate
    return bet, ret, stake


def eval_umaren_only(probs, quin_odds, pay, theta, mode, cap=float("inf"), budget=5000):
    """◎1頭軸ながしの相手のうち馬連 model EV ≥ θ の点のみ。予算を flat/weighted で配分。

    cap: 採用するオッズ上限（穴の暴れ EV を除外する規律。既定 ∞ = 無制限）。
    返り値 (bet, ret, stake)。bet = +EV 点が 1 点以上。清算は実払戻。
    """
    ranked = sorted(probs, key=lambda n: -probs[n])
    if len(ranked) < 2:
        return False, 0, 0
    A = ranked[0]
    sel = []  # (combo, pairprob)
    for p in ranked[1:]:
        c = frozenset({A, p})
        o = quin_odds.get(c, 0.0)
        if o <= 0 or o > cap:
            continue
        pp = p_top2_set(probs, A, p)
        if pp * o >= theta:
            sel.append((c, pp))
    if not sel:
        return False, 0, 0

    combos = [c for c, _ in sel]
    weights = [1.0] * len(sel) if mode == "flat" else [pp for _, pp in sel]
    units = largest_remainder(weights, budget // 100)
    ret = stake = 0
    for c, u in zip(combos, units):
        s = u * 100
        stake += s
        ret += s * pay["umaren"].get(c, 0) // 100
    return True, ret, stake


def eval_umaren_plain(probs, pay, budget=1500):
    """参考: ◎軸 馬連 top5 ながし（EV フィルタ無し, 確率重み, ¥1500）。全鞍機械買い。"""
    ranked = sorted(probs, key=lambda n: -probs[n])
    if len(ranked) < 2:
        return False, 0, 0
    A = ranked[0]
    partners = ranked[1:6]
    combos = [frozenset({A, p}) for p in partners]
    units = largest_remainder([probs[p] for p in partners], budget // 100)
    ret = stake = 0
    for c, u in zip(combos, units):
        s = u * 100
        stake += s
        ret += s * pay["umaren"].get(c, 0) // 100
    return True, ret, stake


def eval_umaren_allflat(probs, pay, budget=5000):
    """apples-to-apples 対照: ◎軸 全頭ながし（EV フィルタ無し・flat・¥5000）。全鞍機械買い。

    umaren_only と universe（全頭）・配分（flat）を揃え、EV フィルタの有無だけを変えた対照。
    これより umaren_only が悪ければ model EV ランキングが逆予測的と切り分けられる。
    """
    ranked = sorted(probs, key=lambda n: -probs[n])
    if len(ranked) < 2:
        return False, 0, 0
    A = ranked[0]
    combos = [frozenset({A, p}) for p in ranked[1:]]
    units = largest_remainder([1.0] * len(combos), budget // 100)
    ret = stake = 0
    for c, u in zip(combos, units):
        s = u * 100
        stake += s
        ret += s * pay["umaren"].get(c, 0) // 100
    return True, ret, stake


# --- 戦略（馬単・exacta）------------------------------------------------------
# 馬単は ◎ を 1 着 or 2 着に固定した順序 2 レッグ (A>p), (p>A) を universe とする
# （= 馬連の「◎軸ながし全頭」を順序空間へ対称拡張）。全レッグが ◎ を含む ◎1頭軸ながしマルチ。
def eval_exacta_only(probs, exacta_odds, pay, theta, mode, cap=float("inf"), budget=5000):
    """◎軸ながしマルチの各順序レッグのうち馬単 model EV ≥ θ の点のみ。予算を flat/weighted で配分。

    cap: 採用するオッズ上限（穴の暴れ EV を除外する規律。既定 ∞ = 無制限）。
    返り値 (bet, ret, stake)。bet = +EV 点が 1 点以上。清算は実払戻。
    """
    ranked = sorted(probs, key=lambda n: -probs[n])
    if len(ranked) < 2:
        return False, 0, 0
    A = ranked[0]
    sel = []  # (combo=(1着,2着), pairprob)
    for p in ranked[1:]:
        for combo in ((A, p), (p, A)):
            o = exacta_odds.get(combo, 0.0)
            if o <= 0 or o > cap:
                continue
            pp = p_exacta(probs, combo[0], combo[1])
            if pp * o >= theta:
                sel.append((combo, pp))
    if not sel:
        return False, 0, 0

    combos = [c for c, _ in sel]
    weights = [1.0] * len(sel) if mode == "flat" else [pp for _, pp in sel]
    units = largest_remainder(weights, budget // 100)
    ret = stake = 0
    for c, u in zip(combos, units):
        s = u * 100
        stake += s
        ret += s * pay["exacta"].get(c, 0) // 100
    return True, ret, stake


def eval_exacta_plain(probs, pay, budget=1500):
    """参考: ◎軸ながしマルチ top5 両方向（EV フィルタ無し, 馬単確率重み, ¥1500）。全鞍機械買い。

    全鞍機械買いの対照なので pay["exacta"] 欠落は損失扱い（JRA は必ず馬単を払うため欠落＝parse 漏れ）。
    """
    ranked = sorted(probs, key=lambda n: -probs[n])
    if len(ranked) < 2:
        return False, 0, 0
    A = ranked[0]
    partners = ranked[1:6]
    combos = [c for p in partners for c in ((A, p), (p, A))]
    weights = [p_exacta(probs, c[0], c[1]) for c in combos]
    units = largest_remainder(weights, budget // 100)
    ret = stake = 0
    for c, u in zip(combos, units):
        s = u * 100
        stake += s
        ret += s * pay["exacta"].get(c, 0) // 100
    return True, ret, stake


def eval_exacta_allflat(probs, pay, budget=5000):
    """apples-to-apples 対照: ◎軸ながしマルチ全頭両方向（EV フィルタ無し・flat・¥5000）。全鞍機械買い。

    eval_exacta_only と universe（全頭両方向）・配分（flat）を揃え、EV フィルタの有無だけを変えた対照。
    これより eval_exacta_only が悪ければ馬単 model EV ランキングが逆予測的と切り分けられる。
    全鞍機械買いの対照なので pay["exacta"] 欠落は損失扱い（JRA は必ず馬単を払うため欠落＝parse 漏れ）。
    """
    ranked = sorted(probs, key=lambda n: -probs[n])
    if len(ranked) < 2:
        return False, 0, 0
    A = ranked[0]
    combos = [c for p in ranked[1:] for c in ((A, p), (p, A))]
    units = largest_remainder([1.0] * len(combos), budget // 100)
    ret = stake = 0
    for c, u in zip(combos, units):
        s = u * 100
        stake += s
        ret += s * pay["exacta"].get(c, 0) // 100
    return True, ret, stake


def eval_pair_leg_swap(probs, quin_odds, exacta_odds, pay, swap=True, budget=1500):
    """ADR 0043 `pick_pair_leg` の直接検証。馬連 top5 ◎軸ながし（確率重み・¥1500）を土台に、
    各ペアで model EV（的中確率×盤面オッズ）が馬単(◎→相手) > 馬連 のとき馬単へ置換する。

    置換は両券種のオッズが present かつ馬単が strict に優位なときだけ（ADR 0043 の発火条件）。
    swap=False なら馬連のみ（土台＝馬連top5 無フィルタと同一）。返り値 (bet, ret, stake, swapped)。
    清算は実払戻。EV 判定は DB 盤面オッズ、清算は result.html 実配当で分離（循環回避）。
    """
    ranked = sorted(probs, key=lambda n: -probs[n])
    if len(ranked) < 2:
        return False, 0, 0, 0
    A = ranked[0]
    partners = ranked[1:6]
    units = largest_remainder([probs[p] for p in partners], budget // 100)
    ret = stake = swapped = 0
    for p, u in zip(partners, units):
        s = u * 100
        stake += s
        qc = frozenset({A, p})
        qo, eo = quin_odds.get(qc, 0.0), exacta_odds.get((A, p), 0.0)
        q_ev = p_top2_set(probs, A, p) * qo
        e_ev = p_exacta(probs, A, p) * eo
        if swap and qo > 0 and eo > 0 and e_ev > q_ev:
            ret += s * pay["exacta"].get((A, p), 0) // 100
            swapped += 1
        else:
            ret += s * pay["umaren"].get(qc, 0) // 100
    return True, ret, stake, swapped


# --- 集計 ---------------------------------------------------------------------
def max_drawdown(pnls):
    """損益列（円, 時系列）の最大ドローダウン（ピークからの最大下落, 正の円）。"""
    peak = cum = dd = 0.0
    for x in pnls:
        cum += x
        peak = max(peak, cum)
        dd = max(dd, peak - cum)
    return dd


# 戦略ラベル列の表示幅（最長ラベル「馬単全頭両方向(無フィルタflat)」=30 セルに合わせる）。
LABEL_W = 30


def pad_disp(label, width=LABEL_W):
    """CJK 全角（East Asian Wide/Fullwidth）を 2 セルとして右側空白パディング（表の列崩れ防止）。"""
    disp = sum(2 if unicodedata.east_asian_width(c) in ("W", "F") else 1 for c in label)
    return label + " " * max(0, width - disp)


def summarize(label, rows):
    """rows = [(ret, stake)]（賭けた鞍のみ, 時系列順）-> 1 行サマリ文字列。"""
    lab = pad_disp(label)
    if not rows:
        return f"{lab} {'0':>4}  {'-':>7} {'-':>5} {'-':>7} {'-':>9}"
    freq = len(rows)
    tot_ret = sum(r for r, _ in rows)
    tot_stake = sum(s for _, s in rows)
    roi = tot_ret / tot_stake * 100 if tot_stake else 0
    hit = sum(1 for r, _ in rows if r > 0) / freq * 100
    per = [r / s * 100 if s else 0 for r, s in rows]
    sd = statistics.pstdev(per) if freq > 1 else 0.0
    dd = max_drawdown([r - s for r, s in rows])
    return f"{lab} {freq:>4}  {roi:>6.1f}% {hit:>4.0f}% {sd:>7.1f} {dd:>9.0f}"


# --- #263: 較正後 model ROI≥100% ゲートの精度診断 ----------------------------
def fav_market(winodds, pid, A):
    """◎（model 1 番手）の市場（人気, 単勝オッズ）。欠落は (None, None)。"""
    pop, odds = winodds.get(pid, {}).get(A, (None, None))
    return pop, odds


def gate_diagnostics(evaluated, winodds, gate=1.0):
    """ゲート通過鞍/非通過鞍を ◎の市場人気度で特徴づける（#263）。

    evaluated 各要素: (date, venue, rnum, pid, probs, quin, trio, exacta, pay)。
    通過鞍の内訳行と、通過 vs 非通過の ◎市場オッズ/人気の平均比較を表示する。
    """
    print(f"=== ゲート診断（model ROI ≥ {gate * 100:.0f}% 通過鞍の内訳）===")
    print(f"{'date':<10} {'場':<3}{'R':>3} {'◎':>3} {'◎model':>7} {'◎mktO':>6} {'◎人':>3} "
          f"{'modelROI':>8} {'実ROI':>7} {'的中':>4}")
    # 鞍数は winodds の有無に依存しない独立カウンタで数える。◎オッズ/人気の平均は
    # winodds がある鞍のみで集計する（欠落鞍を平均から除く）ため、件数とは別に持つ。
    pass_n = fail_n = 0
    pass_odds, fail_odds = [], []
    pass_pop, fail_pop = [], []
    for date, venue, rnum, pid, probs, quin, trio, _exacta, pay in evaluated:
        model_roi, ret, stake = compute_baseline_pf(probs, quin, trio, pay)
        if stake <= 0:
            continue
        A = sorted(probs, key=lambda n: -probs[n])[0]
        pop, odds = fav_market(winodds, pid, A)
        passed = model_roi >= gate
        if passed:
            pass_n += 1
            if odds is not None:
                pass_odds.append(odds)
            if pop is not None:
                pass_pop.append(pop)
        else:
            fail_n += 1
            if odds is not None:
                fail_odds.append(odds)
            if pop is not None:
                fail_pop.append(pop)
        if not passed:
            continue
        real_roi = ret / stake * 100
        odds_s = f"{odds:>6.1f}" if odds is not None else f"{'-':>6}"
        pop_s = f"{pop:>3}" if pop is not None else f"{'-':>3}"
        print(f"{date:<10} {venue:<3}{rnum:>3} {A:>3} {probs[A]:>6.1f}% "
              f"{odds_s} {pop_s} "
              f"{model_roi * 100:>7.1f}% {real_roi:>6.0f}% {'○' if ret > 0 else '×':>3}")

    def _mean(xs):
        return statistics.mean(xs) if xs else float("nan")

    print()
    print(f"通過鞍 {pass_n}（うち◎オッズ有 {len(pass_odds)}）: "
          f"◎平均単勝 {_mean(pass_odds):.2f} 倍 / ◎平均人気 {_mean(pass_pop):.1f}")
    print(f"非通過 {fail_n}（うち◎オッズ有 {len(fail_odds)}）: "
          f"◎平均単勝 {_mean(fail_odds):.2f} 倍 / ◎平均人気 {_mean(fail_pop):.1f}")
    print("（通過/非通過で◎の市場人気度〔単勝オッズ・人気〕が分かれるかの確認用。"
          "71R では両者ともほぼ市場最上位で差が小さく、人気度はゲート通過を分けない）\n")


def gate_sweep(evaluated, winodds, gates, floors):
    """ゲート閾値 × ◎単勝オッズ下限の掃引（#263）。各設定の実 ROI を測る。

    floor は ◎の市場単勝オッズ下限（断然人気の除外）。floor=0 は条件なし。
    ◎オッズが欠落する鞍は floor>0 のとき保守的に除外する。
    """
    print("=== ゲート掃引（model ROI 閾値 × ◎単勝オッズ下限。発動鞍のみ集計）===")
    print(f"{'setting':<{LABEL_W}} {'張鞍':>4}  {'ROI':>7} {'的中':>5} {'σROI':>7} {'maxDD円':>9}")
    for g in gates:
        for fl in floors:
            rows = []
            for _date, _venue, _rnum, pid, probs, quin, trio, _exacta, pay in evaluated:
                model_roi, ret, stake = compute_baseline_pf(probs, quin, trio, pay)
                if stake <= 0 or model_roi < g:
                    continue
                if fl > 0:
                    A = sorted(probs, key=lambda n: -probs[n])[0]
                    _pop, odds = fav_market(winodds, pid, A)
                    if odds is None or odds < fl:
                        continue
                rows.append((ret, stake))
            flabel = "◎O≥%.0f" % fl if fl > 0 else "条件なし"
            print(summarize(f"gate≥{g * 100:.0f}% {flabel}", rows))
        print()


# --- #270: 確率→EV パイプライン（α 市場ブレンド × γ 冪較正）の純 Python 再計算 ------
# Rust 本番パイプライン（src/.../race.rs）の処理順を厳密に鏡映する:
#   m=10 縮約（p_model に焼込み済）→ α ブレンド → γ 冪 → 正規化。
# p_model は α=1.0 実行（ブレンド skip ＝ final=normalize(p_model**γ)）から
# 単一の冪逆変換で復元できる（recover_p_model）。以後は任意の (α, γ) を純 Python で
# 再計算でき、binary を α・γ ごとに再実行せずに同時掃引できる（復元の効率性）。
def market_implied(winodds_pid):
    """単勝オッズ {umaban: (pop, odds)} -> {umaban: implied}（overround 正規化済の市場確率）。

    raw=1/odds、overround=Σraw（オッズのある全頭）、implied=raw/overround。
    odds<=0 の馬は市場確率を持たない（ブレンド対象外）として除外する。
    """
    raw = {}
    for um, (_pop, odds) in winodds_pid.items():
        if odds and odds > 0:
            raw[um] = 1.0 / odds
    s = sum(raw.values())
    if s <= 0:
        return {}
    return {um: r / s for um, r in raw.items()}


def recover_p_model(p_final_pct, gamma=1.25):
    """α=1.0 実行（final=normalize(p_model**γ)）の最終確率%から p_model%を復元する。

    x=(pct/100)**(1/γ); Σ1 正規化; *100。単一の冪逆変換（縮約・ブレンドには触れない）。
    α=1.0 ではブレンド項が消えるので、この逆変換だけで縮約済 p_model を厳密に取り出せる。
    """
    x = {um: (pct / 100.0) ** (1.0 / gamma) for um, pct in p_final_pct.items()}
    s = sum(x.values())
    if s <= 0:
        return {um: 0.0 for um in p_final_pct}
    return {um: v / s * 100.0 for um, v in x.items()}


def recompute_p_final(p_model_pct, implied, alpha, gamma):
    """縮約済 p_model%から (α, γ) で最終確率%を再計算する。Rust の処理順を鏡映。

    model=pct/100; blended=α*model+(1-α)*implied（implied に居る馬のみ、他は model 据置）;
    Σ1 正規化; powered=blended**γ; Σ1 正規化; *100。
    α>=1.0 では (1-α)*implied 項が消え市場補正なし（= normalize(model**γ)）。
    """
    blended = {}
    for um, pct in p_model_pct.items():
        model = pct / 100.0
        if um in implied:
            blended[um] = alpha * model + (1.0 - alpha) * implied[um]
        else:
            blended[um] = model
    s = sum(blended.values())
    if s > 0:
        blended = {um: v / s for um, v in blended.items()}
    powered = {um: v ** gamma for um, v in blended.items()}
    s2 = sum(powered.values())
    if s2 > 0:
        powered = {um: v / s2 for um, v in powered.items()}
    return {um: v * 100.0 for um, v in powered.items()}


# --- 単勝精度メトリクス（answer_check.py と同式。較正が単勝精度を劣化させないかの確認用）---
def top1_hit(probs, winner):
    """probs（{umaban:%}）の最尤馬が実際の勝ち馬なら 1、否なら 0。"""
    if not probs:
        return 0
    pred = max(probs, key=lambda n: probs[n])
    return 1 if pred == winner else 0


def topk_recall(probs, winner, k):
    """勝ち馬が model 上位 k 頭に入れば 1、否なら 0（answer_check.py の recall と同義）。"""
    ranked = sorted(probs, key=lambda n: -probs[n])
    return 1 if winner in ranked[:k] else 0


def brier(probs, winner):
    """単勝 Brier（出走馬平均）= mean_h (p_h/100 - [h==winner])^2。answer_check.py:112-114 と同式。"""
    if not probs:
        return 0.0
    s = 0.0
    for um, pct in probs.items():
        y = 1.0 if um == winner else 0.0
        s += (pct / 100.0 - y) ** 2
    return s / len(probs)


def _avg_ranks(vals):
    """昇順の平均順位（1-based、同値は平均順位）を返す。Spearman 用の内部ヘルパ。"""
    order = sorted(range(len(vals)), key=lambda i: vals[i])
    ranks = [0.0] * len(vals)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and vals[order[j + 1]] == vals[order[i]]:
            j += 1
        avg = (i + j) / 2.0 + 1.0  # 1-based の平均順位
        for k in range(i, j + 1):
            ranks[order[k]] = avg
        i = j + 1
    return ranks


def spearman(xs, ys):
    """Spearman 順位相関（同順位は平均順位）。n<2 または分散ゼロは 0.0。"""
    n = len(xs)
    if n < 2:
        return 0.0
    rx = _avg_ranks(xs)
    ry = _avg_ranks(ys)
    mx = sum(rx) / n
    my = sum(ry) / n
    cov = sum((rx[i] - mx) * (ry[i] - my) for i in range(n))
    vx = sum((r - mx) ** 2 for r in rx)
    vy = sum((r - my) ** 2 for r in ry)
    if vx <= 0 or vy <= 0:
        return 0.0
    return cov / (vx * vy) ** 0.5


def race_winner(pay):
    """実結果の 1 着 umaban を払戻から復元する（馬単キー=(1着,2着) の 1 着）。

    evaluated タプルは top3 を持たないため、着順固定の馬単(exacta)払戻から 1 着を取る。
    馬単欠落 or 同着 1 着（1 着が複数）の鞍は None（top1/Brier 集計から除外）。
    """
    ex = pay.get("exacta", {})
    firsts = {a for (a, _b) in ex}
    return next(iter(firsts)) if len(firsts) == 1 else None


# --- #270: 較正バケット（#249 枠組みと統合）と (α,γ) 同時掃引 -------------------
def calibration_buckets(evaluated, probs_by_race, edges):
    """予測 model ROI でレースをバケット分けし、予測 ROI vs 実現 ROI の較正を測る（#249）。

    probs_by_race: {(date,venue,rnum): 最終確率%}（再計算後）。
    各レースの予測 ROI = compute_baseline_pf(再計算 probs) の model_roi。
    バケット毎に n / 予測ROI平均 / 実現ROI(Σret/Σstake) / 的中率 を出し、
    末尾に Spearman(予測ROI, レース毎実現ROI) と gate>=100% 実現ROI vs 無ゲート平均を付す。

    「逆予測性が解消された」= Spearman>=0（予測 ROI が実現 ROI を正しく順位づける）かつ
    gate>=100% 実現 ROI >= 無ゲート平均（model EV ゲートが正の選別になる）。
    """
    edges = sorted(edges)
    buckets = [[] for _ in range(len(edges) + 1)]
    pred_all, real_all = [], []  # Spearman 用（予測 ROI, レース毎実現 ROI）
    gate_ret = gate_stake = 0
    all_ret = all_stake = 0
    for date, venue, rnum, _pid, _probs, quin, trio, _exacta, pay in evaluated:
        probs = probs_by_race.get((date, venue, rnum))
        if not probs:
            continue
        model_roi, ret, stake = compute_baseline_pf(probs, quin, trio, pay)
        if stake <= 0:
            continue
        bi = 0
        while bi < len(edges) and model_roi >= edges[bi]:
            bi += 1
        buckets[bi].append((model_roi, ret, stake))
        pred_all.append(model_roi)
        real_all.append(ret / stake)
        all_ret += ret
        all_stake += stake
        if model_roi >= 1.0:
            gate_ret += ret
            gate_stake += stake

    print("=== 較正バケット（予測 model ROI 帯 → 実現 ROI, baseline_pf ポートフォリオ）===")
    print(f"{'bucket':<12} {'n':>3} {'予測ROI':>9} {'実現ROI':>9} {'的中率':>7}")
    labels = []
    for i in range(len(edges) + 1):
        lo = edges[i - 1] if i > 0 else None
        hi = edges[i] if i < len(edges) else None
        if lo is None:
            labels.append(f"<{hi * 100:.0f}%")
        elif hi is None:
            labels.append(f">={lo * 100:.0f}%")
        else:
            labels.append(f"{lo * 100:.0f}-{hi * 100:.0f}%")
    for lab, b in zip(labels, buckets):
        labp = pad_disp(lab, 12)
        if not b:
            print(f"{labp} {0:>3} {'-':>9} {'-':>9} {'-':>7}")
            continue
        n = len(b)
        mean_pred = sum(m for m, _, _ in b) / n * 100
        sr = sum(r for _, r, _ in b)
        ss = sum(s for _, _, s in b)
        real = sr / ss * 100 if ss else 0.0
        hit = sum(1 for _, r, _ in b if r > 0) / n * 100
        print(f"{labp} {n:>3} {mean_pred:>8.1f}% {real:>8.1f}% {hit:>6.0f}%")

    sp = spearman(pred_all, real_all)
    gate_roi = gate_ret / gate_stake * 100 if gate_stake else float("nan")
    nogate_roi = all_ret / all_stake * 100 if all_stake else float("nan")
    print(f"\nSpearman(予測ROI, レース実現ROI) = {sp:+.3f}"
          "（>=0 なら予測が実現を順位づけ＝逆予測性が解消）")
    print(f"gate>=100% 実現ROI {gate_roi:.1f}% vs 無ゲート平均 {nogate_roi:.1f}%"
          "（gate>=無ゲート なら model EV ゲートが正の選別）\n")


def joint_sweep(evaluated, winodds, p_models, alphas, gammas):
    """(α, γ) 同時掃引。各 (α,γ) で全レースを再計算→ baseline_pf 清算→ ゲート整合性を測る。

    p_models: {(date,venue,rnum): 縮約済 p_model%}（α=1.0 実行から復元）。
    行: n_gate(model_roi>=1.0) / gated実現ROI / 無ゲート実現ROI / 差分 /
        Spearman(model_roi, ret/stake) / top1率 / 平均Brier。
    delta>=0 かつ Spearman>=0 なら、その (α,γ) で model EV ゲートが正直な選別に戻る。
    """
    print("=== (α, γ) 同時掃引（baseline_pf ポートフォリオのゲート整合性 + 単勝精度）===")
    print(f"{'alpha':>5} {'gamma':>5} {'n_gate':>6} {'gateROI':>8} {'noGate':>7} "
          f"{'delta':>6} {'Spear':>6} {'top1':>5} {'Brier':>6}")
    for alpha in alphas:
        for gamma in gammas:
            preds, reals = [], []
            g_ret = g_stake = a_ret = a_stake = 0
            top1_n = top1_tot = 0
            briers = []
            for date, venue, rnum, pid, _probs, quin, trio, _exacta, pay in evaluated:
                pm = p_models.get((date, venue, rnum))
                if not pm:
                    continue
                implied = market_implied(winodds.get(pid, {}))
                probs = recompute_p_final(pm, implied, alpha, gamma)
                model_roi, ret, stake = compute_baseline_pf(probs, quin, trio, pay)
                if stake <= 0:
                    continue
                preds.append(model_roi)
                reals.append(ret / stake)
                a_ret += ret
                a_stake += stake
                if model_roi >= 1.0:
                    g_ret += ret
                    g_stake += stake
                w = race_winner(pay)
                if w is not None:
                    top1_tot += 1
                    top1_n += top1_hit(probs, w)
                    briers.append(brier(probs, w))
            n_gate = sum(1 for m in preds if m >= 1.0)
            gate_roi = g_ret / g_stake * 100 if g_stake else float("nan")
            nogate_roi = a_ret / a_stake * 100 if a_stake else float("nan")
            delta = gate_roi - nogate_roi if (g_stake and a_stake) else float("nan")
            sp = spearman(preds, reals)
            t1 = top1_n / top1_tot * 100 if top1_tot else float("nan")
            mb = sum(briers) / len(briers) if briers else float("nan")
            print(f"{alpha:>5.2f} {gamma:>5.2f} {n_gate:>6} {gate_roi:>7.1f}% {nogate_roi:>6.1f}% "
                  f"{delta:>+6.1f} {sp:>+6.3f} {t1:>4.0f}% {mb:>6.4f}")
        print()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--races", default="/tmp/bt250/bt_races.tsv")
    ap.add_argument("--pred-dir", default="/tmp/bt250")
    ap.add_argument("--results-dir", default="/tmp/bt250")
    ap.add_argument("--exotic-odds", default="/tmp/bt250/bt_exotic_odds.tsv")
    ap.add_argument("--winodds", default="/tmp/bt250/bt_winodds.tsv",
                    help="単勝オッズ TSV（#263 ゲート診断の市場人気度。欠落可）")
    ap.add_argument("--ev-grid", default="1.0,1.2,1.5")
    ap.add_argument("--cap-grid", default="inf,50,30", help="馬連オッズ上限の掃引（inf=無制限）")
    ap.add_argument("--gate-grid", default="0,1.0,1.1,1.2,1.3",
                    help="#263 baseline_pf ゲート閾値（model ROI）の掃引。0=無ゲート対照")
    ap.add_argument("--odds-floor-grid", default="0,2,3,5",
                    help="#263 ◎単勝オッズ下限の掃引（0=条件なし）")
    # #270: 確率→EV パイプライン（α×γ）の同時再検証。--p-model-dir は α=1.0 実行の bt_pred dir。
    ap.add_argument("--p-model-dir", default=None,
                    help="#270 α=1.0 実行の bt_pred dir（指定時のみ較正バケット+α×γ掃引を追加）")
    ap.add_argument("--alpha-grid", default="0,0.1,0.2,0.3,0.5,1.0",
                    help="#270 α（市場ブレンド）掃引。α=市場へ寄せる係数＝高いほど市場補正は弱い")
    ap.add_argument("--gamma-grid", default="1.0,1.1,1.25,1.5,2.0",
                    help="#270 γ（冪較正）掃引。γ>1 で上位確率に質量を集中")
    ap.add_argument("--bucket-edges", default="0.8,0.9,1.0,1.1,1.2",
                    help="#270 較正バケットの予測 model ROI 境界")
    args = ap.parse_args()

    races = parse_races(args.races)
    exotic = parse_exotic(args.exotic_odds)
    winodds = parse_winodds(args.winodds)
    preds = {}
    for d in sorted({r["date"] for r in races}):
        p = Path(args.pred_dir) / f"bt_pred_{d}.txt"
        if p.exists():
            preds[d] = parse_pred(p)

    thetas = [float(x) for x in args.ev_grid.split(",")]
    caps = [(float(x), "cap∞" if float(x) == float("inf") else f"cap{int(float(x))}")
            for x in args.cap_grid.split(",")]

    # 各レースを 1 度評価。exotic オッズ（馬連/三連複盤面）が無い鞍は対象外。
    # exacta 盤面の欠落は eval_exacta_only 内で個別スキップ（共通母集団 used を維持するため）。
    evaluated = []  # (date, venue, rnum, pid, probs, quin, trio, exacta, pay)
    skips = dict(probs=0, exotic=0, result=0)
    for r in sorted(races, key=lambda x: (x["date"], x["venue"], x["rnum"])):
        probs = preds.get(r["date"], {}).get((r["venue"], r["rnum"]))
        ex = exotic.get(r["pid"])
        resf = Path(args.results_dir) / f"res_{r['nk']}.html"
        if not probs:
            skips["probs"] += 1
            continue
        if not ex or not ex["quinella"]:
            skips["exotic"] += 1
            continue
        if not resf.exists():
            skips["result"] += 1
            continue
        top3, pay = parse_result(resf)
        if len(top3) < 3:
            skips["result"] += 1
            continue
        evaluated.append(
            (r["date"], r["venue"], r["rnum"], r["pid"], probs,
             ex["quinella"], ex["trio"], ex["exacta"], pay)
        )

    used = len(evaluated)
    print(
        f"対象レース: {used}（スキップ {sum(skips.values())}: "
        f"probs欠落 {skips['probs']} / exoticオッズ欠落 {skips['exotic']} / result欠落 {skips['result']}）\n"
    )

    # baseline_pf（全鞍評価し、発動ゲート通過鞍のみ集計）
    base_rows = []
    for _date, _venue, _rnum, _pid, probs, quin, trio, _exacta, pay in evaluated:
        bet, ret, stake = eval_baseline_pf(probs, quin, trio, pay)
        if bet:
            base_rows.append((ret, stake))

    # 参考: 無フィルタ対照（馬連 top5 / 馬連全頭 / 馬単 top5 両方向 / 馬単全頭両方向, 全鞍機械買い）
    um_plain_rows, um_allflat_rows = [], []
    ex_plain_rows, ex_allflat_rows = [], []
    for _date, _venue, _rnum, _pid, probs, _quin, _trio, _exacta, pay in evaluated:
        bet, ret, stake = eval_umaren_plain(probs, pay)
        if bet:
            um_plain_rows.append((ret, stake))
        bet, ret, stake = eval_umaren_allflat(probs, pay)
        if bet:
            um_allflat_rows.append((ret, stake))
        bet, ret, stake = eval_exacta_plain(probs, pay)
        if bet:
            ex_plain_rows.append((ret, stake))
        bet, ret, stake = eval_exacta_allflat(probs, pay)
        if bet:
            ex_allflat_rows.append((ret, stake))

    # maxDD(円) は戦略間で予算が異なる（baseline_pf ¥3,500 / top5 ¥1,500 / allflat・特化 ¥5,000）ため
    # スケール非不変＝直接比較不可。判定は ROI / 的中 / σ（いずれもスケール不変）で行うこと。
    print(f"=== 戦略比較（全 {used}R に機械適用、発動ゲート通過鞍のみ集計）===")
    print(f"{'strategy':<{LABEL_W}} {'張鞍':>4}  {'ROI':>7} {'的中':>5} {'σROI':>7} {'maxDD円':>9}")
    print(summarize("baseline_pf", base_rows))
    print(summarize("馬連top5(無フィルタ)", um_plain_rows))
    print(summarize("馬連全頭(無フィルタflat)", um_allflat_rows))
    print(summarize("馬単top5両方向(無フィルタ)", ex_plain_rows))
    print(summarize("馬単全頭両方向(無フィルタflat)", ex_allflat_rows))
    print()

    # ADR 0043 pick_pair_leg の直接検証: 馬連 top5 を土台に、EV 優位ペアのみ馬単(◎→相手)へ置換。
    # 土台（swap無）= 馬連top5無フィルタと同値。置換版が土台を上回れば順序プレミアムは実エッジ。
    base_leg, swap_leg, swap_count = [], [], 0
    for _date, _venue, _rnum, _pid, probs, quin, _trio, exacta, pay in evaluated:
        _, ret, stake, _ = eval_pair_leg_swap(probs, quin, exacta, pay, swap=False)
        base_leg.append((ret, stake))
        _, ret, stake, sw = eval_pair_leg_swap(probs, quin, exacta, pay, swap=True)
        swap_leg.append((ret, stake))
        swap_count += sw
    print(f"=== ADR 0043 順序プレミアム直接検証（馬連top5 vs 馬単EV優位ペア置換, 全{used}R）===")
    print(f"{'strategy':<{LABEL_W}} {'張鞍':>4}  {'ROI':>7} {'的中':>5} {'σROI':>7} {'maxDD円':>9}")
    print(summarize("ペア脚: 馬連のみ(土台)", base_leg))
    print(summarize(f"ペア脚: 馬単置換({swap_count}脚)", swap_leg))
    print()

    # #263: 較正後 model ROI≥100% ゲートの精度診断と閾値/オッズ条件の掃引。
    gate_diagnostics(evaluated, winodds, gate=1.0)
    gates = [float(x) for x in args.gate_grid.split(",")]
    floors = [float(x) for x in args.odds_floor_grid.split(",")]
    gate_sweep(evaluated, winodds, gates, floors)

    # 馬連特化（EV≥θ）。cap=∞（課題の素案）と規律版（オッズ上限）を掃引。
    for mode in ("flat", "weighted"):
        for cap, clabel in caps:
            for theta in thetas:
                rows = []
                for _date, _venue, _rnum, _pid, probs, quin, _trio, _exacta, pay in evaluated:
                    bet, ret, stake = eval_umaren_only(probs, quin, pay, theta, mode, cap)
                    if bet:
                        rows.append((ret, stake))
                print(summarize(f"umaren θ={theta:.1f} {clabel} {mode}", rows))
        print()

    # 馬単特化（EV≥θ）。順序プレミアム検証。馬連と同一の θ×cap×flat/weighted 掃引。
    for mode in ("flat", "weighted"):
        for cap, clabel in caps:
            for theta in thetas:
                rows = []
                for _date, _venue, _rnum, _pid, probs, _quin, _trio, exacta, pay in evaluated:
                    bet, ret, stake = eval_exacta_only(probs, exacta, pay, theta, mode, cap)
                    if bet:
                        rows.append((ret, stake))
                print(summarize(f"exacta θ={theta:.1f} {clabel} {mode}", rows))
        print()

    # #270: 確率→EV パイプライン（α×γ）の同時再検証（--p-model-dir 指定時のみ）。
    # α=1.0 実行の最終確率から p_model を復元し、任意 (α,γ) を純 Python で再計算する。
    if args.p_model_dir:
        a1_preds = {}
        for d in sorted({r["date"] for r in races}):
            p = Path(args.p_model_dir) / f"bt_pred_{d}.txt"
            if p.exists():
                a1_preds[d] = parse_pred(p)
        p_models = {}  # (date,venue,rnum) -> 縮約済 p_model%
        for date, venue, rnum, _pid, _probs, *_rest in evaluated:
            pf = a1_preds.get(date, {}).get((venue, rnum))
            if pf:
                p_models[(date, venue, rnum)] = recover_p_model(pf, gamma=1.25)

        # production (α=0.2, γ=1.25) で各レースの最終確率を再計算。
        prod_probs = {}
        for date, venue, rnum, pid, _probs, *_rest in evaluated:
            pm = p_models.get((date, venue, rnum))
            if pm:
                implied = market_implied(winodds.get(pid, {}))
                prod_probs[(date, venue, rnum)] = recompute_p_final(pm, implied, 0.2, 1.25)

        # SANITY: 再計算 (α=0.2,γ=1.25) が本番 bt_pred（--pred-dir）を 1 桁丸め内で再現するか。
        max_abs = 0.0
        nchk = 0
        for date, venue, rnum, _pid, _probs, *_rest in evaluated:
            rp = prod_probs.get((date, venue, rnum))
            pp = preds.get(date, {}).get((venue, rnum))
            if not rp or not pp:
                continue
            for um in pp:
                if um in rp:
                    max_abs = max(max_abs, abs(rp[um] - pp[um]))
                    nchk += 1
        if nchk:
            print(f"=== #270 SANITY: 再計算(α=0.2,γ=1.25) vs 本番 bt_pred "
                  f"（{nchk}頭・最大絶対誤差 {max_abs:.3f}pt）===")
            print("（純 Python 再計算が Rust 本番を鏡映していれば 1 桁丸め由来の ~0.1-0.3pt 以内）\n")

        # 較正バケットの production headline は本番 bt_pred の実 probs を使う
        # （ADR 0044 の gate_sweep と同値＝24.5% に一致。recompute は上の SANITY 専用）。
        edges = [float(x) for x in args.bucket_edges.split(",")]
        prod_actual = {(d, v, r): pr for d, v, r, _pid, pr, *_rest in evaluated}
        calibration_buckets(evaluated, prod_actual, edges)
        alphas = [float(x) for x in args.alpha_grid.split(",")]
        gammas = [float(x) for x in args.gamma_grid.split(",")]
        joint_sweep(evaluated, winodds, p_models, alphas, gammas)


if __name__ == "__main__":
    main()
