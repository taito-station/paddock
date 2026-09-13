"""umaren_box_backtest.py のユニットテスト（pytest 不要・`python3 test_umaren_box_backtest.py`）.

#629: 印馬の馬連ボックスを配分に足す案の backtest ハーネスの不変量テスト。
ボックス組合せ生成・配分計算・settle の正当性を検証する。
"""
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))


# --- box combination tests ---

def test_box_pairs_6heads():
    """6 頭のボックス → C(6,2) = 15 ペア"""
    import umaren_box_backtest as U
    heads = [1, 2, 3, 4, 5, 6]
    pairs = U.box_pairs(heads)
    assert len(pairs) == 15
    assert all(len(p) == 2 for p in pairs)
    assert len(set(pairs)) == 15  # 重複なし


def test_box_pairs_5heads():
    """5 頭のボックス → C(5,2) = 10 ペア"""
    import umaren_box_backtest as U
    heads = [1, 2, 3, 4, 5]
    pairs = U.box_pairs(heads)
    assert len(pairs) == 10


def test_box_pairs_2heads():
    """2 頭 → 1 ペアのみ"""
    import umaren_box_backtest as U
    pairs = U.box_pairs([1, 2])
    assert len(pairs) == 1
    assert pairs[0] == frozenset({1, 2})


# --- allocation tests ---

def test_uniform_alloc_exact():
    """¥1,500 / 15 点 = 各 ¥100（均等割り切り）"""
    import umaren_box_backtest as U
    alloc = U.uniform_alloc(budget_yen=1500, n_points=15)
    assert alloc == [1] * 15
    assert sum(alloc) * 100 == 1500


def test_uniform_alloc_remainder():
    """¥1,500 / 10 点 = 各 ¥100 で ¥500 余る → 先頭 5 点に追加"""
    import umaren_box_backtest as U
    alloc = U.uniform_alloc(budget_yen=1500, n_points=10)
    assert sum(alloc) * 100 == 1500
    assert all(u >= 1 for u in alloc)


def test_uniform_alloc_insufficient():
    """予算が全点に ¥100 配れない → 可能な分だけ（¥500 / 15 点 → 5 点のみ）"""
    import umaren_box_backtest as U
    alloc = U.uniform_alloc(budget_yen=500, n_points=15)
    assert sum(alloc) * 100 == 500
    assert sum(1 for u in alloc if u > 0) == 5


# --- frequency (A) metric ---

def test_frequency_a_positive():
    """1-2着が印6頭内 AND ◎が top3 外 → True"""
    import umaren_box_backtest as U
    probs = {1: 30, 2: 20, 3: 15, 4: 10, 5: 8, 6: 7, 7: 5, 8: 5}
    marked = U.get_marked_horses(probs, n_partners=5)
    assert len(marked) == 6  # axis + 5 partners
    top3 = [3, 6, 7]  # ◎=1 は top3 に不在
    first, second = 3, 6  # 両方 marked 内
    axis = max(probs, key=lambda n: probs[n])
    result = U.is_box_opportunity(axis, marked, first, second, top3)
    assert result is True


def test_frequency_a_negative_axis_in_top3():
    """◎が top3 に入っている → False（ながしで拾えるから box 不要）"""
    import umaren_box_backtest as U
    probs = {1: 30, 2: 20, 3: 15, 4: 10, 5: 8, 6: 7}
    marked = U.get_marked_horses(probs, n_partners=5)
    axis = max(probs, key=lambda n: probs[n])
    top3 = [1, 3, 6]  # ◎=1 が top3 に入っている
    result = U.is_box_opportunity(axis, marked, 3, 6, top3)
    assert result is False


def test_frequency_a_negative_12_outside():
    """1-2着が marked 外 → False"""
    import umaren_box_backtest as U
    probs = {1: 30, 2: 20, 3: 15, 4: 10, 5: 8, 6: 7, 7: 5, 8: 3}
    marked = U.get_marked_horses(probs, n_partners=5)
    axis = max(probs, key=lambda n: probs[n])
    top3 = [7, 8, 4]  # ◎=1 は top3 外
    result = U.is_box_opportunity(axis, marked, 7, 8, top3)
    assert result is False  # 7,8 は marked 外


# --- settle variant tests ---

def test_settle_baseline_nonkonsen():
    """非混戦時の baseline: wide/umaren/sanrenpuku 全てながし"""
    import umaren_box_backtest as U
    probs = {1: 25, 2: 20, 3: 18, 4: 17, 5: 10, 6: 5, 7: 3, 8: 2}
    pay = {"umaren": {}, "wide": {}, "trio": {}}
    top3 = [2, 3, 5]
    ret, stake = U.settle_baseline(probs, top3, pay, konsen=False)
    assert stake == 5000
    assert ret == 0  # 払戻なし（pay が空）


def test_settle_baseline_konsen():
    """混戦時の baseline: 3連複ボックス追加 + 券種予算が減額"""
    import umaren_box_backtest as U
    probs = {1: 25, 2: 24, 3: 23, 4: 18, 5: 5, 6: 3, 7: 1, 8: 1}
    # band_of: 0.70*25=17.5 以上 → {1,2,3,4} = 4頭 → konsen
    pay = {"umaren": {}, "wide": {}, "trio": {}}
    top3 = [2, 3, 5]
    ret, stake = U.settle_baseline(probs, top3, pay, konsen=True)
    assert stake == 5000
    assert ret == 0


def test_settle_box_variant_payout():
    """馬連ボックスの的中ケース"""
    import umaren_box_backtest as U
    probs = {1: 30, 2: 20, 3: 15, 4: 10, 5: 8, 6: 7}
    marked = U.get_marked_horses(probs, n_partners=5)
    pay_umaren = {frozenset({3, 6}): 9900}
    ret, stake = U.settle_umaren_box(marked, pay_umaren, budget_yen=1500, n_heads=6)
    assert stake > 0
    assert ret > 0  # frozenset({3,6}) に命中


def test_settle_variant_konsen_fallback():
    """混戦時の variant は baseline と同じ挙動にフォールバック"""
    import umaren_box_backtest as U
    probs = {1: 25, 2: 24, 3: 23, 4: 18, 5: 5, 6: 3, 7: 1, 8: 1}
    marked = U.get_marked_horses(probs, n_partners=5)
    pay = {"umaren": {}, "wide": {}, "trio": {}}
    top3 = [2, 3, 5]
    ret_bl, stake_bl = U.settle_baseline(probs, top3, pay, konsen=True)
    ret_v, stake_v = U.settle_variant(probs, marked, top3, pay, True, "wide", 6)
    assert ret_v == ret_bl
    assert stake_v == stake_bl


def test_settle_variant_replaces_correct_leg():
    """非混戦で replace_leg="umaren" のとき umaren だけ box に置換される"""
    import umaren_box_backtest as U
    probs = {1: 30, 2: 20, 3: 15, 4: 10, 5: 8, 6: 7, 7: 5, 8: 3}
    marked = U.get_marked_horses(probs, n_partners=5)
    pay = {"umaren": {frozenset({3, 6}): 5000}, "wide": {}, "trio": {}}
    top3 = [3, 6, 5]
    ret_bl, _ = U.settle_baseline(probs, top3, pay, konsen=False)
    ret_v, _ = U.settle_variant(probs, marked, top3, pay, False, "umaren", 6)
    # variant は box で frozenset({3,6}) を拾えるので baseline と異なる
    assert ret_v != ret_bl
    assert ret_v > 0


def test_get_marked_horses():
    """印馬 = ◎(axis) + top5 partners。先頭が axis、以降は勝率降順"""
    import umaren_box_backtest as U
    probs = {1: 30, 2: 20, 3: 15, 4: 10, 5: 8, 6: 7, 7: 5, 8: 3}
    marked = U.get_marked_horses(probs, n_partners=5)
    assert len(marked) == 6
    assert marked[0] == 1  # axis
    assert marked[1:] == [2, 3, 4, 5, 6]  # 勝率降順
    assert 8 not in marked


if __name__ == "__main__":
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    passed = failed = 0
    for t in tests:
        try:
            t()
            passed += 1
            print(f"  PASS  {t.__name__}")
        except Exception as e:
            failed += 1
            print(f"  FAIL  {t.__name__}: {e}")
    print(f"\n{passed} passed, {failed} failed")
    sys.exit(1 if failed else 0)
