#!/usr/bin/env python3
"""kelly_compare の単体テスト（#316）。Kelly 式の Rust 鏡映・配分丸め・bankroll sim を検証。"""
import sys
from itertools import combinations
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import kelly_compare as kc  # noqa: E402


def test_kelly_fraction_matches_rust():
    # Rust tests.rs と同じ数値: p=0.05, gross=21 → b=20, Kelly=(0.05*20-0.95)/20=0.0025。
    assert abs(kc.kelly_fraction(0.05, 21.0) - 0.0025) < 1e-9
    # 教科書例: p=0.5, gross=3.0 → b=2, f=(0.5*2-0.5)/2=0.25。
    assert abs(kc.kelly_fraction(0.5, 3.0) - 0.25) < 1e-9


def test_kelly_fraction_negative_edge_is_zero():
    # EV<1（p·gross<1）は負 edge → 0。p=0.1, gross=5 → EV=0.5。
    assert kc.kelly_fraction(0.1, 5.0) == 0.0
    # オッズ≤1（払戻なし）も 0。
    assert kc.kelly_fraction(0.9, 1.0) == 0.0


def test_kelly_fraction_cap():
    # 強い edge は cap でクランプ。p=0.9, gross=3.0 → f=(0.9*2-0.1)/2=0.85 → cap 0.25。
    assert kc.kelly_fraction(0.9, 3.0, cap=0.25) == 0.25


def test_stake_fixed_total_is_budget():
    # 総額は方式に依らず ¥3,500 ちょうど（端数なし・100円単位）。
    probs = {1: 40.0, 2: 20.0, 3: 15.0, 4: 10.0, 5: 8.0, 6: 7.0}
    A = 1
    quin = {frozenset({A, p}): 5.0 for p in (2, 3, 4, 5, 6)}
    trio = {frozenset({A, a, b}): 30.0 for a, b in combinations((2, 3, 4, 5, 6), 2)}
    pay = {"umaren": {}, "trio": {}, "wide": {}, "exacta": {}}
    for w in ("prob", "kelly"):
        _ret, stake = kc.stake_fixed(probs, quin, trio, pay, w)
        assert stake == kc.PF_BUDGET, (w, stake)
        assert stake % 100 == 0


def test_sim_bankroll_flat_settles_real_payout():
    # 単一レース・的中で flat 清算が実払戻で残高更新されるか。
    probs = {1: 40.0, 2: 20.0, 3: 15.0, 4: 10.0, 5: 8.0, 6: 7.0}
    A = 1
    quin = {frozenset({A, p}): 5.0 for p in (2, 3, 4, 5, 6)}
    trio = {frozenset({A, a, b}): 30.0 for a, b in combinations((2, 3, 4, 5, 6), 2)}
    # 馬連 1-2 が 1000 円/¥100（=10倍）で的中、他は外れ。
    pay = {"umaren": {frozenset({1, 2}): 1000}, "trio": {}, "wide": {}, "exacta": {}}
    series, rows, n = kc.sim_bankroll([(probs, quin, trio, pay)], "flat", 100000)
    assert n == 1
    ret, stake = rows[0]
    assert stake == kc.PF_BUDGET
    # 残高 = b0 + ret - stake。ret>0（馬連的中分）。
    assert series[-1] == 100000 + ret - stake
    assert ret > 0


def test_sim_bankroll_kelly_skips_when_no_positive_edge():
    # 全脚が負 edge（EV<1）なら Kelly は張らず残高据え置き。
    probs = {1: 40.0, 2: 20.0, 3: 15.0, 4: 10.0, 5: 8.0, 6: 7.0}
    A = 1
    # オッズを低くして全脚 EV<1 に（p_top2 はせいぜい ~0.2、odds=1.5 → EV<1）。
    quin = {frozenset({A, p}): 1.5 for p in (2, 3, 4, 5, 6)}
    trio = {frozenset({A, a, b}): 1.5 for a, b in combinations((2, 3, 4, 5, 6), 2)}
    pay = {"umaren": {}, "trio": {}, "wide": {}, "exacta": {}}
    series, rows, n = kc.sim_bankroll([(probs, quin, trio, pay)], "kelly", 100000)
    assert n == 0
    assert series[-1] == 100000


def test_sim_bankroll_kelly_wins_increase_bankroll():
    # Kelly が +EV 脚を張り、的中したら実払戻で残高が増えることを検証（vacuous 回避）。
    probs = {1: 50.0, 2: 25.0, 3: 15.0, 4: 10.0}
    A = 1
    # 高オッズで +EV 脚を確実に作る（p_top2(1,2)≈0.33、odds=6 → EV≈2.0）。
    quin = {frozenset({A, p}): 6.0 for p in (2, 3, 4)}
    trio = {frozenset({A, a, b}): 40.0 for a, b in combinations((2, 3, 4), 2)}
    # 馬連 1-2 が 600 円/¥100（=6倍）で的中。
    pay = {"umaren": {frozenset({1, 2}): 600}, "trio": {}, "wide": {}, "exacta": {}}
    series, rows, n = kc.sim_bankroll([(probs, quin, trio, pay)], "kelly", 100000, lam=1.0)
    assert n == 1
    ret, stake = rows[0]
    assert stake > 0 and stake % 100 == 0
    assert ret > 0  # 馬連的中分の払戻
    assert series[-1] == 100000 + ret - stake


def test_ruin_prob_full_kelly_is_deterministic_and_high():
    # 確率較正不良を模した負け続けのレース列で full Kelly(λ=1) の破産率が高い。
    # seed 固定で決定的（回帰テスト）。全レース外れ → bankroll は単調減少。
    probs = {1: 50.0, 2: 25.0, 3: 15.0, 4: 10.0}
    A = 1
    quin = {frozenset({A, p}): 6.0 for p in (2, 3, 4)}
    trio = {frozenset({A, a, b}): 40.0 for a, b in combinations((2, 3, 4), 2)}
    pay = {"umaren": {}, "trio": {}, "wide": {}, "exacta": {}}  # 全外れ
    races = [(probs, quin, trio, pay)] * 10
    rp = kc.ruin_prob(races, "kelly", 100000, 1.0, 0.2, 100)
    assert rp == 1.0  # 全外れ・full Kelly はどの順でも破産水準へ


def test_sim_bankroll_kelly_round_unit():
    # Kelly stake は 100 円単位で丸められる。
    probs = {1: 50.0, 2: 25.0, 3: 15.0, 4: 10.0}
    A = 1
    quin = {frozenset({A, p}): 6.0 for p in (2, 3, 4)}
    trio = {frozenset({A, a, b}): 40.0 for a, b in combinations((2, 3, 4), 2)}
    pay = {"umaren": {}, "trio": {}, "wide": {}, "exacta": {}}
    _series, rows, n = kc.sim_bankroll([(probs, quin, trio, pay)], "kelly", 100000, lam=1.0)
    # 高オッズ +EV 脚があるので必ず張る（vacuous 回避）。
    assert n == 1
    _ret, stake = rows[0]
    assert stake % 100 == 0
