#!/usr/bin/env python3
"""#316: fractional Kelly 配分 vs 現行ヒューリスティック配分の walk-forward 比較。

確率モデルのレバーは枯渇（ADR 0052/0053）。残る別軸が「賭け方（staking）」。現行の買い目配分は
経験則（最大剰余法・固定予算・最低¥100, CLAUDE.md「買い方ルール」）で、確率とオッズに対する
理論最適配分（fractional Kelly）になっていない。本スクリプトは **同一の買い目候補（baseline_pf =
馬連◎軸ながし top5 + 三連複◎軸ながし top5）・同一の当時オッズ** の上で配分方式だけを変え、
リーク無し walk-forward（/tmp/bt252, 71R）で現行配分と Kelly 配分を比較する。

二つの土俵で測る:
  (A) 定額土俵 — 総賭金を現行と同額（¥3,500/レース）に固定し、券種内の重みだけを
      確率重み（現行）→ Kelly 比率 に替える。総額一定なので実 ROI 差は「どの脚に厚く張るか」
      だけに由来する apples-to-apples 比較（alloc_compare.py と同じ原則）。
  (B) bankroll 土俵 — fractional Kelly の本質（edge に応じ総賭金を bankroll 比で変える）を測る。
      開始資金から時系列に張り→実払戻で清算→残高更新。現行（固定 ¥3,500/レース flat 張り）と
      Kelly（λ·Σf_leg·bankroll）を、最終資金倍率・σ・最大DD・近似破産確率で比較する。
      破産確率はレース順の順列並べ替え（permutation・非復元リサンプル。既定 2000 本）で
      残高が ruin 閾値に到達した経路の割合として近似する。固定 71R を並べ替えるだけなので
      捉えるのは「賭ける順序による経路リスク」で、母集団からのサンプリング変動ではない
      （標本変動 CI ではない）。最終資金倍率・実 ROI は実時系列（chronological）1 経路の実現値。

【循環回避】EV/Kelly 判定に使うオッズ（DB 盤面）と的中時の清算（result.html 実配当）を分離する
（umaren_backtest.py と同方針）。Kelly 式は本番 Rust src/domain/src/betting/kelly.rs を鏡映:
net odds b=gross-1, f=(p·b−q)/b, clamp。

リーク注意: bt_pred の確率は analyze predict 由来で過去走較正にリークの可能性（memory
alpha_sign_and_predict_leak）。ただし配分比較は全方式が同一 probs を共有する common-mode で、
相対比較（Kelly vs 現行）には影響しない。

使い方:
    python3 scripts/predict-check/kelly_compare.py [--bt-dir /tmp/bt252]
"""
import argparse
import random
import statistics
import sys
from itertools import combinations
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import umaren_backtest as ub  # noqa: E402

UMAREN_BUDGET = 1500
TRIO_BUDGET = 2000
PF_BUDGET = UMAREN_BUDGET + TRIO_BUDGET  # 現行 baseline_pf の固定総額 ¥3,500


def kelly_fraction(p, gross_odds, cap=1.0):
    """Rust src/domain/src/betting/kelly.rs を鏡映。net b=gross-1; f=(p·b−q)/b; clamp[0,cap]。

    gross_odds = 払戻倍率（JRA オッズ）。EV=p·gross_odds。負 edge / オッズ≤1 は 0。
    """
    b = gross_odds - 1.0
    if b <= 0:
        return 0.0
    q = 1.0 - p
    f = (p * b - q) / b
    return max(0.0, min(cap, f))


def race_legs(probs, quin_odds, trio_odds, pay):
    """baseline_pf ユニバース（馬連◎軸 top5 + 三連複◎軸 top5）の脚を返す。

    各脚 = (種別, combo, prob 的中確率, gross_odds DB盤面, pay_raw 実配当)。
    pay_raw = result.html の確定配当（¥100 当たり円・整数）。清算は flat と同じ整数演算
    `s * pay_raw // 100`（s は 100 の倍数）。pay/100.0 を先に float 化すると割り切れない配当
    （例 230→2.3 は float 非正確）で int() が 1 円下振れするため、生の整数で持つ。
    DB 盤面オッズ欠落の脚は Kelly では除外（edge 不明な脚は張らない）。
    """
    ranked = sorted(probs, key=lambda n: -probs[n])
    if len(ranked) < 3:
        return []
    A = ranked[0]
    partners = ranked[1:6]
    legs = []
    for p in partners:
        c = frozenset({A, p})
        o = quin_odds.get(c, 0.0)
        if o > 0:
            legs.append(("umaren", c, ub.p_top2_set(probs, A, p), o,
                         pay["umaren"].get(c, 0)))
    for a, b in combinations(partners, 2):
        c = frozenset({A, a, b})
        o = trio_odds.get(c, 0.0)
        if o > 0:
            legs.append(("trio", c, ub.p_top3_set(probs, (A, a, b)), o,
                         pay["trio"].get(c, 0)))
    return legs


def load(bt_dir):
    """alloc_compare.load と同形だが、Kelly の edge 算出に DB 盤面オッズも返す。

    返り値: races = [(probs, quin_odds, trio_odds, pay)]（date,venue,rnum 昇順＝時系列）。
    """
    races = ub.parse_races(str(Path(bt_dir) / "bt_races.tsv"))
    exotic = ub.parse_exotic(str(Path(bt_dir) / "bt_exotic_odds.tsv"))
    preds = {}
    for d in sorted({r["date"] for r in races}):
        p = Path(bt_dir) / f"bt_pred_{d}.txt"
        if p.exists():
            preds[d] = ub.parse_pred(p)
    out = []
    skips = dict(probs=0, exotic=0, result=0)
    for r in sorted(races, key=lambda x: (x["date"], x["venue"], x["rnum"])):
        probs = preds.get(r["date"], {}).get((r["venue"], r["rnum"]))
        ex = exotic.get(r["pid"])
        resf = Path(bt_dir) / f"res_{r['nk']}.html"
        if not probs:
            skips["probs"] += 1
            continue
        if not ex or not ex["quinella"]:
            skips["exotic"] += 1
            continue
        if not resf.exists():
            skips["result"] += 1
            continue
        top3, pay = ub.parse_result(resf)
        if len(top3) < 3:
            skips["result"] += 1
            continue
        out.append((probs, ex["quinella"], ex["trio"], pay))
    return out, skips


# --- (A) 定額土俵: 総額 ¥3,500 固定で重みだけ変える ---------------------------
def stake_fixed(probs, quin_odds, trio_odds, pay, weight):
    """総額 ¥3,500（馬連¥1,500+三連複¥2,000）固定で配分し (ret, stake) を返す。

    weight="prob": 現行（馬連=probs[p]/三連複=probs[a]·probs[b]、最低¥100）。
    weight="kelly": 券種内を Kelly 比率で配分（最低¥100は現行と揃え weighting 効果を分離）。
    清算は実払戻 pay。配分母数 minu=1 は現行と同一（floor 効果を交絡させない）。

    留保: +EV 脚が 1 本も無いレースでは Kelly 重みが全 0 になり、largest_remainder の
    「重み和≤0 → 一様」フォールバックで ¥3,500 を均等張りに縮退する（Kelly本来の「張らない」
    挙動とは乖離）。定額土俵は「毎レース ¥3,500 を必ず張る前提で重みだけ比較する」枠組みゆえの
    仕様で、Kelly の自己縮小（張らない判断）は (B) bankroll 土俵が忠実に測る。
    """
    ranked = sorted(probs, key=lambda n: -probs[n])
    if len(ranked) < 3:
        return 0, 0
    A = ranked[0]
    partners = ranked[1:6]
    um_combos = [frozenset({A, p}) for p in partners]
    tr_pairs = list(combinations(partners, 2))
    tr_combos = [frozenset({A, a, b}) for a, b in tr_pairs]

    if weight == "prob":
        um_w = [probs[p] for p in partners]
        tr_w = [probs[a] * probs[b] for a, b in tr_pairs]
    else:  # kelly
        um_w = [kelly_fraction(ub.p_top2_set(probs, A, p), quin_odds.get(frozenset({A, p}), 0.0))
                for p in partners]
        tr_w = [kelly_fraction(ub.p_top3_set(probs, (A, a, b)), trio_odds.get(frozenset({A, a, b}), 0.0))
                for a, b in tr_pairs]

    um_stakes = [u * 100 for u in ub.largest_remainder(um_w, UMAREN_BUDGET // 100, minu=1)]
    tr_stakes = [u * 100 for u in ub.largest_remainder(tr_w, TRIO_BUDGET // 100, minu=1)]
    ret = stake = 0
    for c, s in zip(um_combos, um_stakes):
        stake += s
        ret += s * pay["umaren"].get(c, 0) // 100
    for c, s in zip(tr_combos, tr_stakes):
        stake += s
        ret += s * pay["trio"].get(c, 0) // 100
    return ret, stake


def fixed_table(races):
    print(f"=== (A) 定額土俵: 総額¥{PF_BUDGET}/R 固定・重みのみ変更（全{len(races)}R 機械買い）===")
    print(f"{'method':<22} {'実ROI':>7} {'的中':>5} {'σROI':>7} {'総賭金':>9} {'総払戻':>9}")
    for label, w in [("現行(確率重み)", "prob"), ("Kelly重み", "kelly")]:
        rows = []
        for probs, quin, trio, pay in races:
            ret, stake = stake_fixed(probs, quin, trio, pay, w)
            if stake > 0:
                rows.append((ret, stake))
        tot_ret = sum(r for r, _ in rows)
        tot_stake = sum(s for _, s in rows)
        roi = tot_ret / tot_stake * 100 if tot_stake else 0
        hit = sum(1 for r, _ in rows if r > 0) / len(rows) * 100 if rows else 0
        per = [r / s * 100 for r, s in rows if s]
        sd = statistics.pstdev(per) if len(per) > 1 else 0.0
        print(f"{ub.pad_disp(label, 22)} {roi:>6.1f}% {hit:>4.0f}% {sd:>7.1f} {tot_stake:>9} {tot_ret:>9}")
    print()


# --- (B) bankroll 土俵: flat（現行）vs fractional Kelly --------------------------
def sim_bankroll(races, mode, b0, lam=0.5, kelly_cap=1.0, round_unit=100):
    """時系列 races を逐次清算し bankroll 系列を返す。

    mode="flat": 各レース固定 ¥PF_BUDGET を現行重みで張る（残高不足なら張らない＝skip）。
    mode="kelly": 各レース W=λ·Σf_leg·bankroll を +EV 脚へ Kelly 比率配分（W は残高上限）。
    round_unit: 100円単位丸め（実運用制約）。残高は清算後に更新。

    近似の留保:
    - 脚ごとの f_leg を単純加算する（Σf_leg）。同一レースの馬連・三連複は ◎を共有し結果が
      相互排他/相関するため、真の同時 Kelly はこれより小さい。素朴加算は過大張り側に振れ、
      λ=1 の破産規模を増幅しうる（＝「Kelly は危険」という棄却結論には保守的な向き）。
    - 100円丸めで稀に Σstake>bank になる鞍は当該レースを張らず skip する（保守的な過小張り。
      張り回数がわずかに減るだけで結論には影響しない）。

    返り値: (bankroll_series, per_race_ret 配列[(ret,stake)], n_bet)。
    bankroll_series[0]=b0、以後レース毎の清算後残高。
    """
    bank = b0
    series = [b0]
    rows = []
    for probs, quin, trio, pay in races:
        if mode == "flat":
            ret, stake = stake_fixed(probs, quin, trio, pay, "prob")
            if stake > bank:  # 残高不足は張れない
                series.append(bank)
                continue
        else:  # kelly
            legs = race_legs(probs, quin, trio, pay)
            sized = [(c, kelly_fraction(p, o, kelly_cap), pr) for _typ, c, p, o, pr in legs]
            sized = [(c, f, pr) for c, f, pr in sized if f > 0]
            if not sized:
                series.append(bank)
                continue
            fsum = sum(f for _c, f, _pr in sized)
            total_frac = min(1.0, lam * fsum)
            wager = total_frac * bank
            stake = 0
            ret = 0
            for _c, f, pr in sized:
                s = wager * f / fsum
                s = int(round(s / round_unit)) * round_unit  # 100円単位
                if s <= 0:
                    continue
                stake += s
                ret += s * pr // 100  # 実払戻（flat と同一の整数演算）
            if stake <= 0 or stake > bank:
                series.append(bank)
                continue
        bank += ret - stake
        series.append(bank)
        rows.append((ret, stake))
        if bank <= 0:
            break
    return series, rows, len(rows)


def ruin_prob(races, mode, b0, lam, ruin_frac, n_perm, seed=42):
    """レース順の順列並べ替え（permutation・非復元）で残高が b0·ruin_frac 以下に到達する経路割合。

    固定 races を shuffle するだけなので「賭ける順序による経路リスク」の近似で、復元リサンプリング
    （統計的ブートストラップ）ではない。母集団のサンプリング変動 CI ではなく、λ=1 が順序に依らず
    破産する（破産率 100%）といった経路頑健性の判定に用いる。"""
    rng = random.Random(seed)
    ruin_level = b0 * ruin_frac
    hit = 0
    for _ in range(n_perm):
        order = races[:]
        rng.shuffle(order)
        series, _rows, _n = sim_bankroll(order, mode, b0, lam=lam)
        if min(series) <= ruin_level:
            hit += 1
    return hit / n_perm if n_perm else float("nan")


def bankroll_table(races, b0, lambdas, ruin_frac, n_perm):
    print(f"=== (B) bankroll 土俵: 開始¥{b0:,}・{len(races)}R 時系列・実払戻清算 ===")
    print(f"{'strategy':<20} {'最終資金':>10} {'倍率':>6} {'実ROI':>7} {'的中':>5} "
          f"{'σ%':>6} {'maxDD%':>7} {'最小残':>9} {'破産%':>6}")

    def report(label, mode, lam):
        series, rows, _n = sim_bankroll(races, mode, b0, lam=lam)
        final = series[-1]
        mult = final / b0
        tot_ret = sum(r for r, _ in rows)
        tot_stake = sum(s for _, s in rows)
        roi = tot_ret / tot_stake * 100 if tot_stake else 0
        hit = sum(1 for r, _ in rows if r > 0) / len(rows) * 100 if rows else 0
        per = [r / s * 100 for r, s in rows if s]
        sd = statistics.pstdev(per) if len(per) > 1 else 0.0
        peak = b0
        dd = 0.0
        for v in series:
            peak = max(peak, v)
            dd = max(dd, (peak - v) / peak * 100 if peak > 0 else 0.0)
        rp = ruin_prob(races, mode, b0, lam, ruin_frac, n_perm) * 100
        print(f"{ub.pad_disp(label, 20)} {final:>10,.0f} {mult:>5.2f}x {roi:>6.1f}% {hit:>4.0f}% "
              f"{sd:>6.1f} {dd:>6.1f}% {min(series):>9,.0f} {rp:>5.1f}%")

    report("現行(flat ¥3,500)", "flat", 0.0)
    for lam in lambdas:
        report(f"Kelly λ={lam:g}", "kelly", lam)
    print(f"\n（最終資金/倍率/ROI/的中/σ/maxDD/最小残=実時系列1経路の実現値。"
          f"破産%＝レース順{n_perm}本の順列リサンプル(permutation・非復元)で"
          f"残高≤開始×{ruin_frac:g}到達経路の割合）")
    print()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bt-dir", default="/tmp/bt252", help="#252 手順で生成した入力ディレクトリ")
    ap.add_argument("--b0", type=int, default=100000, help="bankroll 開始資金（円）")
    ap.add_argument("--lambdas", default="0.25,0.5,1.0", help="fractional Kelly 係数の掃引")
    ap.add_argument("--ruin-frac", type=float, default=0.2, help="破産判定の残高水準（開始比）")
    ap.add_argument("--n-perm", type=int, default=2000,
                    help="破産確率近似の順列リサンプル(permutation・非復元)本数")
    args = ap.parse_args()

    races, skips = load(args.bt_dir)
    print(f"対象 {len(races)}R（baseline_pf ユニバース固定・配分方式のみ比較。"
          f"除外 {sum(skips.values())}: probs欠落 {skips['probs']} / "
          f"exoticオッズ欠落 {skips['exotic']} / result欠落 {skips['result']}）\n")
    fixed_table(races)
    lambdas = [float(x) for x in args.lambdas.split(",")]
    bankroll_table(races, args.b0, lambdas, args.ruin_frac, args.n_perm)


if __name__ == "__main__":
    main()
