"""live_ev.py の数値ロジック最小テスト（pytest 不要・`python3 test_live_ev.py` で実行）.

確率計算（Plackett-Luce）と配分（最大剰余法）はデグレに気付きにくいので、
不変量（合計・対称性・縮退）を assert で固定する。
"""
from itertools import combinations

import live_ev as L


def approx(a, b, eps=1e-9):
    return abs(a - b) < eps


def test_largest_remainder():
    # 合計が units にちょうど一致し、各点が minu 以上
    al = L.largest_remainder([1, 1, 1], 6)
    assert sum(al) == 6 and al == [2, 2, 2], al
    al = L.largest_remainder([3, 1], 4, minu=1)
    assert sum(al) == 4 and min(al) >= 1, al
    # 重い順に厚く配分される
    al = L.largest_remainder([5, 1], 10, minu=1)
    assert al[0] > al[1] and sum(al) == 10, al
    # 縮退: units < minu*n は各点に1ずつを units 個だけ置く（合計=units）
    al = L.largest_remainder([1, 1, 1, 1, 1], 3, minu=1)
    assert sum(al) == 3 and set(al) <= {0, 1}, al


def test_plackett_luce_invariants():
    # 4頭の不均等フィールド
    probs = {1: 40.0, 2: 30.0, 3: 20.0, 4: 10.0}
    horses = list(probs)
    # 馬連: 全ペアの「ともに上位2着」確率の総和は 1（ちょうど1ペアが上位2）
    s2 = sum(L.p_top2_set(probs, a, b) for a, b in combinations(horses, 2))
    assert approx(s2, 1.0), s2
    # 3連複: 全トリオの「上位3着を占める」確率の総和は 1
    s3 = sum(L.p_top3_set(probs, t) for t in combinations(horses, 3))
    assert approx(s3, 1.0), s3
    # 対称性: 順序を変えても同じ
    assert approx(L.p_top2_set(probs, 1, 2), L.p_top2_set(probs, 2, 1))
    assert approx(L.p_top3_set(probs, (1, 2, 3)), L.p_top3_set(probs, (3, 1, 2)))
    # 確率は [0,1]
    assert all(0.0 <= L.p_pair_top3(probs, a, b) <= 1.0 for a, b in combinations(horses, 2))


def test_three_horse_field_certain():
    # 3頭ちょうどなら、その3頭が必ず上位3着 → 各ペアのワイド的中=1、トリオ=1
    probs = {1: 50.0, 2: 30.0, 3: 20.0}
    assert approx(L.p_top3_set(probs, (1, 2, 3)), 1.0)
    for a, b in combinations(probs, 2):
        assert approx(L.p_pair_top3(probs, a, b), 1.0)


def test_konsen_band():
    # ◎の0.70倍以上が◎含め4頭以上 → 混戦
    konsen = {1: 30.0, 2: 25.0, 3: 22.0, 4: 21.5, 5: 5.0}
    assert L.is_konsen(konsen) and L.band_of(konsen)[0] == 1
    # 抜けた1強 → 非混戦
    clear = {1: 50.0, 2: 12.0, 3: 10.0, 4: 8.0, 5: 6.0}
    assert not L.is_konsen(clear)


def test_parse_pred():
    # predict 出力フォーマット契約を固定（ヘッダ + 「馬番 馬名 勝率% 連対% 複勝%」行）
    import tempfile
    import os
    sample = (
        "--- レース 6: tokyo 芝 1600m ---\n"
        "   1 サンプルウマ              33.6%    33.6%    33.6%\n"
        "   2 テストホース              12.1%    20.0%    28.0%\n"
        "\n"
        "--- レース 7: hanshin ダート 1800m ---\n"
        "  10 ベツノウマ                25.0%    40.0%    55.0%\n"
    )
    fd, path = tempfile.mkstemp(suffix=".txt")
    try:
        with os.fdopen(fd, "w") as f:
            f.write(sample)
        out = L.parse_pred(path)
        assert set(out) == {("tokyo", 6), ("hanshin", 7)}, list(out)
        assert out[("tokyo", 6)]["probs"] == {1: 33.6, 2: 12.1}, out[("tokyo", 6)]
        assert out[("tokyo", 6)]["surface"] == "芝" and out[("tokyo", 6)]["dist"] == 1600
        assert out[("hanshin", 7)]["probs"] == {10: 25.0}
    finally:
        os.unlink(path)


def test_build_bets_budget():
    # 買い目の総額が予算（100円単位に丸めた額）ちょうど。5,000非倍数や混戦でも取りこぼさない。
    clear = {1: 35.0, 2: 15.0, 3: 12.0, 4: 8.0, 5: 6.0, 6: 5.0}
    konsen = {1: 30.0, 2: 25.0, 3: 22.0, 4: 21.0, 5: 8.0, 6: 6.0}
    for probs in (clear, konsen):
        for budget in (5000, 10000, 3300, 7777, 4900):
            _, _, _, bets = L.build_bets(probs, budget)
            assert sum(amt for _, _, amt in bets) == budget // 100 * 100, (budget, bets)


def test_build_bets_scale_invariant():
    # build_bets はスケール非依存（gen_predictions.py は [0,1]、live_ev は百分率で渡す）。
    # 組合せ・各点の金額・合計がすべてスケールに依存しないことを確認する。
    pct = {1: 35.0, 2: 15.0, 3: 12.0, 4: 8.0, 5: 6.0, 6: 5.0}
    frac = {k: v / 100 for k, v in pct.items()}
    _, _, _, bets_pct = L.build_bets(pct, 5000)
    _, _, _, bets_frac = L.build_bets(frac, 5000)
    assert bets_pct == bets_frac  # 組合せ・個別金額ともに完全一致
    assert sum(a for _, _, a in bets_pct) == 5000


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")


if __name__ == "__main__":
    main()
