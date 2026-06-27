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
            a, b = (int(x) for x in key.split(">"))
            slot[bt][(a, b)] = float(odds)
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
    """参考: ◎軸ながしマルチ top5 両方向（EV フィルタ無し, 馬単確率重み, ¥1500）。全鞍機械買い。"""
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


def summarize(label, rows):
    """rows = [(ret, stake)]（賭けた鞍のみ, 時系列順）-> 1 行サマリ文字列。"""
    if not rows:
        return f"{label:<24} {'0':>4}  {'-':>7} {'-':>5} {'-':>7} {'-':>9}"
    freq = len(rows)
    tot_ret = sum(r for r, _ in rows)
    tot_stake = sum(s for _, s in rows)
    roi = tot_ret / tot_stake * 100 if tot_stake else 0
    hit = sum(1 for r, _ in rows if r > 0) / freq * 100
    per = [r / s * 100 if s else 0 for r, s in rows]
    sd = statistics.pstdev(per) if freq > 1 else 0.0
    dd = max_drawdown([r - s for r, s in rows])
    return f"{label:<24} {freq:>4}  {roi:>6.1f}% {hit:>4.0f}% {sd:>7.1f} {dd:>9.0f}"


# --- #263: 較正後 model ROI≥100% ゲートの精度診断 ----------------------------
def fav_market(winodds, pid, A):
    """◎（model 1 番手）の市場（人気, 単勝オッズ）。欠落は (None, None)。"""
    pop, odds = winodds.get(pid, {}).get(A, (None, None))
    return pop, odds


def gate_diagnostics(evaluated, winodds, gate=1.0):
    """ゲート通過鞍/非通過鞍を ◎の市場人気度で特徴づける（#263）。

    evaluated 各要素: (date, venue, rnum, pid, probs, quin, trio, pay)。
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
    print(f"{'setting':<22} {'張鞍':>4}  {'ROI':>7} {'的中':>5} {'σROI':>7} {'maxDD円':>9}")
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
    print(f"{'strategy':<24} {'張鞍':>4}  {'ROI':>7} {'的中':>5} {'σROI':>7} {'maxDD円':>9}")
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
    print(f"{'strategy':<24} {'張鞍':>4}  {'ROI':>7} {'的中':>5} {'σROI':>7} {'maxDD円':>9}")
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


if __name__ == "__main__":
    main()
