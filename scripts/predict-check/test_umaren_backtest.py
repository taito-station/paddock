"""umaren_backtest.py の馬単(exacta)拡張の最小テスト（pytest 不要・`python3 test_umaren_backtest.py`）.

#262 で追加した順序付き Plackett-Luce（p_exacta）と入力パース（exacta は '>' 順序キー）の
不変量・契約を assert で固定する。デグレに気付きにくい数値ロジックの回帰検出が目的。
"""
import os
import tempfile
from itertools import permutations

import umaren_backtest as U


def approx(a, b, eps=1e-9):
    return abs(a - b) < eps


def test_p_exacta_invariants():
    # 不均等 4 頭フィールド
    probs = {1: 40.0, 2: 30.0, 3: 20.0, 4: 10.0}
    horses = list(probs)
    # 全順序ペア（1着>2着）の総和は 1（ちょうど 1 つの順序ペアが上位 2 着を占める）
    s = sum(U.p_exacta(probs, a, b) for a, b in permutations(horses, 2))
    assert approx(s, 1.0), s
    # 馬連との整合: 順序 2 レッグの和 == 無順 馬連確率
    for a, b in permutations(horses, 2):
        assert approx(U.p_exacta(probs, a, b) + U.p_exacta(probs, b, a), U.p_top2_set(probs, a, b))
    # 非対称性: 強い馬が 1 着の順序の方が確率が高い
    assert U.p_exacta(probs, 1, 4) > U.p_exacta(probs, 4, 1)
    # [0,1] かつ縮退ガード（z-pa<=0 で 0）
    assert all(0.0 <= U.p_exacta(probs, a, b) <= 1.0 for a, b in permutations(horses, 2))
    assert U.p_exacta({1: 100.0}, 1, 1) == 0.0  # z-pa==0 ガード


def test_parse_exotic_exacta_ordered():
    # exacta は '>' 順序キー→(1着,2着)タプル、無順券種は '-'→frozenset
    sample = (
        "pid1\tquinella\t3-5\t12.4\n"
        "pid1\ttrio\t3-5-7\t88.0\n"
        "pid1\texacta\t3>5\t25.1\n"
        "pid1\texacta\t5>3\t40.7\n"
        "pid1\tunknown\t9-9\t1.0\n"  # 想定外 bet_type は無視される
    )
    fd, path = tempfile.mkstemp(suffix=".tsv")
    try:
        with os.fdopen(fd, "w") as f:
            f.write(sample)
        out = U.parse_exotic(path)
    finally:
        os.unlink(path)
    slot = out["pid1"]
    assert slot["quinella"][frozenset({3, 5})] == 12.4
    assert slot["trio"][frozenset({3, 5, 7})] == 88.0
    # 順序が保持され、向きで別エントリになる
    assert slot["exacta"][(3, 5)] == 25.1
    assert slot["exacta"][(5, 3)] == 40.7
    assert (5, 3) != (3, 5)


def test_eval_exacta_only_uses_ordered_payout():
    # ◎=1。(1>2) のみ +EV になるオッズ盤面で、的中は実払戻(1,2)から取れること。
    probs = {1: 50.0, 2: 30.0, 3: 20.0}
    exacta_odds = {(1, 2): 100.0, (2, 1): 1.0, (1, 3): 1.0, (3, 1): 1.0}
    pay = {"umaren": {}, "wide": {}, "trio": {}, "exacta": {(1, 2): 10000}}
    bet, ret, stake = U.eval_exacta_only(probs, exacta_odds, pay, theta=1.0, mode="flat")
    assert bet and stake == 5000
    # (1>2) が当たり、(1,2) の払戻でリターンが発生する
    assert ret == 5000 // 100 * 10000
    # exacta 盤面が空なら個別スキップ（bet=False）
    bet2, _, _ = U.eval_exacta_only(probs, {}, pay, theta=1.0, mode="flat")
    assert not bet2


def test_parse_result_exacta_ordered_key():
    # 馬単(Umatan)行は (1着,2着) タプルでキー化、無順券種(Umaren)は frozenset
    html = (
        '<tr class="Umaren"><td class="Result"><ul><li>6</li><li>4</li></ul></td>'
        '<td class="Payout">3,830円</td></tr>'
        '<tr class="Umatan"><td class="Result"><ul><li>6</li><li>4</li></ul></td>'
        '<td class="Payout">7,210円</td></tr>'
    )
    fd, path = tempfile.mkstemp(suffix=".html")
    try:
        with os.fdopen(fd, "w") as f:
            f.write(html)
        _, pay = U.parse_result(path)
    finally:
        os.unlink(path)
    assert pay["umaren"][frozenset({6, 4})] == 3830
    assert pay["exacta"][(6, 4)] == 7210  # 出現順=1着>2着
    assert (4, 6) not in pay["exacta"]  # 逆順は別物


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")


if __name__ == "__main__":
    main()
