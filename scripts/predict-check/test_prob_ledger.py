"""prob_ledger.py のユニットテスト（#719）。

対象: 割引 Harville の組合せ確率（馬連・連対・ワイド・3連複）を総当たりの独立実装と照合 /
和の恒等式 / λ=1 での umaren_backtest との一致 / λ≠1 での harville_lambda_fit.stage_loglik との一致 /
券種別の母集合（同着・着順欠落）/ レース単位のスコアと一様比 / floor / 外部確率 TSV の読み込み /
4 端点の窓検証 / 系統の母集合揃え / ブートストラップ CI / ledger 追記。
"""

import math
import os
import subprocess
import sys
import tempfile
from collections import OrderedDict
from itertools import combinations, permutations

# CI の predict-check ジョブ（stdlib-only）では numpy/pytest が無いため自己スキップする
# （test_prob_eval.py と同じ扱い）。
try:
    import numpy as np
    import pytest
except ImportError:
    if __name__ == "__main__":
        print("skip: numpy/pytest 不在（stdlib-only CI では対象外。ローカルは pytest で実行）")
        sys.exit(0)
    raise

import harville_lambda_fit as hl
import prob_ledger as pl
import umaren_backtest as ub

P5 = [0.40, 0.25, 0.15, 0.12, 0.08]
L2, L3 = 0.8, 0.6


# ---------- 総当たりの独立実装（テスト側の参照値） ----------


def _ordered3(p, i, j, k, l2, l3):
    """順序付き 1-2-3 着確率を定義どおり 1 項ずつ計算する。"""
    z1 = sum(p)
    z2 = sum(p[m] ** l2 for m in range(len(p)) if m != i)
    z3 = sum(p[m] ** l3 for m in range(len(p)) if m not in (i, j))
    return p[i] / z1 * p[j] ** l2 / z2 * p[k] ** l3 / z3


def _exacta(p, i, j, l2):
    z1 = sum(p)
    z2 = sum(p[m] ** l2 for m in range(len(p)) if m != i)
    return p[i] / z1 * p[j] ** l2 / z2


def _ref(p, l2, l3):
    n = len(p)
    quinella = {(i, j): _exacta(p, i, j, l2) + _exacta(p, j, i, l2) for i, j in combinations(range(n), 2)}
    top2 = [p[i] / sum(p) + sum(_exacta(p, j, i, l2) for j in range(n) if j != i) for i in range(n)]
    trio = {
        t: sum(_ordered3(p, a, b, c, l2, l3) for a, b, c in permutations(t))
        for t in combinations(range(n), 3)
    }
    wide = {
        (i, j): sum(v for t, v in trio.items() if i in t and j in t) for i, j in combinations(range(n), 2)
    }
    return quinella, top2, wide, trio


# ---------- 組合せ確率 ----------


def test_combo_matches_bruteforce_discounted_5horses():
    c = pl.combo_probs(np.array(P5), L2, L3)
    quinella, top2, wide, trio = _ref(P5, L2, L3)
    for (i, j), v in quinella.items():
        assert c.quinella[i, j] == pytest.approx(v)
        assert c.quinella[j, i] == pytest.approx(v)
    for i, v in enumerate(top2):
        assert c.top2[i] == pytest.approx(v)
    for (i, j), v in wide.items():
        assert c.wide[i, j] == pytest.approx(v)
    for (i, j, k), v in trio.items():
        for a, b, d in permutations((i, j, k)):
            assert c.trio[a, b, d] == pytest.approx(v)


def test_combo_sums_are_identities():
    # 馬連 Σ=1 / 連対 Σ=2 / ワイド Σ=3 / 3連複 Σ=1（割引ありでも成り立つ）。
    for p, l2, l3 in ((P5, L2, L3), (P5, 1.0, 1.0), ([0.5, 0.3, 0.2], 0.7, 0.5)):
        c = pl.combo_probs(np.array(p), l2, l3)
        n = len(p)
        iu = np.triu_indices(n, 1)
        assert c.quinella[iu].sum() == pytest.approx(1.0)
        assert c.top2.sum() == pytest.approx(2.0)
        assert c.wide[iu].sum() == pytest.approx(3.0)
        assert sum(c.trio[t] for t in combinations(range(n), 3)) == pytest.approx(1.0)
        assert c.win.sum() == pytest.approx(1.0)


def test_combo_normalizes_unnormalized_input():
    c1 = pl.combo_probs(np.array(P5), L2, L3)
    c2 = pl.combo_probs(np.array(P5) * 3.0, L2, L3)
    assert np.allclose(c1.quinella, c2.quinella)
    assert np.allclose(c1.trio, c2.trio)


def test_lambda1_matches_umaren_backtest():
    probs = {i: v for i, v in enumerate(P5)}
    c = pl.combo_probs(np.array(P5), 1.0, 1.0)
    for a, b in combinations(range(5), 2):
        assert c.quinella[a, b] == pytest.approx(ub.p_top2_set(probs, a, b))
    for t in combinations(range(5), 3):
        assert c.trio[t] == pytest.approx(ub.p_top3_set(probs, t))


def test_ordered_log_matches_stage_loglik():
    # ln P(1-2-3 着の順序) = ln p_1着 + λ2 段の尤度 + λ3 段の尤度（harville_lambda_fit と同じ定義）。
    rows = [{"race_id": "R1", "model_win_pure": v} for v in P5]
    podium = OrderedDict([("R1", (rows, 2, 0, 4))])
    expected = (
        math.log(P5[2] / sum(P5))
        + hl.stage_loglik(podium, "model_win_pure", 2, L2)
        + hl.stage_loglik(podium, "model_win_pure", 3, L3)
    )
    c = pl.combo_probs(np.array(P5), L2, L3)
    assert math.log(c.ordered3(2, 0, 4)) == pytest.approx(expected)


def test_combo_regression_literals_discounted():
    # 総当たりの参照実装（_ref）と独立に、代表値をリテラルで固定する（参照実装側の同時誤りも検出する）。
    # 馬連(0,1) は手計算でも 0.4·0.25^0.8/(Σp^0.8 − 0.4^0.8) + 0.25·0.4^0.8/(Σp^0.8 − 0.25^0.8) ≈ 0.1525 + 0.1183。
    c = pl.combo_probs(np.array(P5), L2, L3)
    assert c.quinella[0, 1] == pytest.approx(0.2708006828711557)
    assert c.quinella[3, 4] == pytest.approx(0.02578486057063056)
    assert c.top2[0] == pytest.approx(0.6635518109465817)
    assert c.wide[0, 1] == pytest.approx(0.5668363476149145)
    assert c.trio[0, 1, 2] == pytest.approx(0.23517446108051054)


def test_small_race_literal_value():
    # 3 頭 p=[.5,.3,.2]・λ=1: 馬連(0,1) = .5*.3/.5 + .3*.5/.7
    c = pl.combo_probs(np.array([0.5, 0.3, 0.2]), 1.0, 1.0)
    assert c.quinella[0, 1] == pytest.approx(0.3 + 0.15 / 0.7)


def test_combo_rejects_bad_probs():
    for bad in ([0.5, -0.1, 0.6], [0.0, 0.0, 0.0], [0.5, float("nan"), 0.5]):
        with pytest.raises(ValueError):
            pl.combo_probs(np.array(bad), 1.0, 1.0)


# ---------- 着順・母集合 ----------


def test_podium_per_position():
    assert pl.podium([3, 1, 2, None, 4]) == {1: 1, 2: 2, 3: 0}
    # 3 着同着: 1・2 着は一意、3 着は無し
    assert pl.podium([1, 2, 3, 3, 5]) == {1: 0, 2: 1, 3: None}
    # 1 着同着: 2 着は存在しない
    assert pl.podium([1, 1, 3, 4, 5]) == {1: None, 2: None, 3: 2}


def test_race_scores_exclusion_by_bet_type():
    c = pl.combo_probs(np.array(P5), L2, L3)
    # 3 着同着: 単勝・馬連・連対は残り、ワイド・3連複は除外
    s = pl.race_scores(c, [1, 2, 3, 3, 5])
    assert s["win"] is not None and s["quinella"] is not None and s["top2"] is not None
    assert s["wide"] is None and s["trio"] is None
    # 1 着同着: 単勝・馬連・連対も除外（ワイド・3連複も 1〜3 着が一意でないので除外）
    s = pl.race_scores(c, [1, 1, 3, 4, 5])
    assert all(s[k] is None for k in ("win", "quinella", "top2", "wide", "trio"))
    # 着順なしの馬（中止）がいても表彰台が確定していれば残す（連対ラベルは 0）
    s = pl.race_scores(c, [2, 1, None, 3, 4])
    assert all(s[k] is not None for k in ("win", "quinella", "top2", "wide", "trio"))


def test_race_scores_values_and_uniform():
    p = np.array(P5)
    c = pl.combo_probs(p, L2, L3)
    fin = [3, 1, 5, 2, 4]  # 1着=idx1, 2着=idx3, 3着=idx0
    s = pl.race_scores(c, fin)
    n = 5
    assert s["win"] == (pytest.approx(-math.log(c.win[1])), pytest.approx(math.log(n)))
    assert s["quinella"][0] == pytest.approx(-math.log(c.quinella[1, 3]))
    assert s["quinella"][1] == pytest.approx(math.log(math.comb(n, 2)))
    hits = [(0, 1), (0, 3), (1, 3)]
    assert s["wide"][0] == pytest.approx(np.mean([-math.log(c.wide[a, b]) for a, b in hits]))
    assert s["wide"][1] == pytest.approx(math.log(math.comb(n, 2) / 3))
    assert s["trio"][0] == pytest.approx(-math.log(c.trio[0, 1, 3]))
    assert s["trio"][1] == pytest.approx(math.log(math.comb(n, 3)))
    # 連対: 馬単位の二値 log-loss の和・馬数・Brier の和
    y = np.array([0, 1, 0, 1, 0], dtype=float)
    q = c.top2
    ll = -np.sum(y * np.log(q) + (1 - y) * np.log(1 - q))
    assert s["top2"]["ll_sum"] == pytest.approx(ll)
    assert s["top2"]["n"] == 5
    assert s["top2"]["brier_sum"] == pytest.approx(np.sum((q - y) ** 2))
    # 馬連・ワイドのペア単位 Brier（副指標）
    iu = list(combinations(range(n), 2))
    yq = np.array([1.0 if (a, b) == (1, 3) else 0.0 for a, b in iu])
    pq = np.array([c.quinella[a, b] for a, b in iu])
    assert s["quinella_pairs"]["brier_sum"] == pytest.approx(np.sum((pq - yq) ** 2))
    assert s["quinella_pairs"]["n"] == 10


def test_race_scores_top2_override():
    c = pl.combo_probs(np.array(P5), L2, L3)
    override = np.array([0.7, 0.5, 0.3, 0.3, 0.2])
    s = pl.race_scores(c, [3, 1, 5, 2, 4], p_top2=override)
    y = np.array([0, 1, 0, 1, 0], dtype=float)
    ll = -np.sum(y * np.log(override) + (1 - y) * np.log(1 - override))
    assert s["top2"]["ll_sum"] == pytest.approx(ll)
    # 馬連は override の影響を受けない
    assert s["quinella"][0] == pytest.approx(-math.log(c.quinella[1, 3]))


def test_race_scores_floor_is_counted():
    # 勝者に確率 0 → floor して件数を数える（log-loss が無限大にならない）
    c = pl.combo_probs(np.array([0.6, 0.4, 0.0]), 1.0, 1.0)
    s = pl.race_scores(c, [2, 3, 1])
    assert math.isfinite(s["win"][0])
    assert s["floored"] >= 1


def test_uniform_probs_give_zero_pseudo_r2():
    n = 6
    c = pl.combo_probs(np.full(n, 1.0 / n), 1.0, 1.0)
    s = pl.race_scores(c, [1, 2, 3, 4, 5, 6])
    for k in ("win", "quinella", "wide", "trio"):
        nll, uni = s[k]
        assert nll == pytest.approx(uni)
    assert s["top2"]["ll_sum"] == pytest.approx(s["top2"]["uni_sum"])


def test_exclusion_reasons_by_bet_group():
    assert pl.exclusion_reasons([1, 2, 3, 4]) == {"win": None, "quinella": None, "wide": None}
    # 3 着同着: ワイド・3連複だけ外れる
    assert pl.exclusion_reasons([1, 2, 3, 3]) == {"win": None, "quinella": None, "wide": "dead_heat"}
    # 1 着同着 → 2 着が存在しない: 同着を優先して数える
    assert pl.exclusion_reasons([1, 1, 3, 4]) == {"win": "dead_heat", "quinella": "dead_heat", "wide": "dead_heat"}
    # 着順が取れていない（結果未取込）
    assert pl.exclusion_reasons([None] * 4) == {"win": "missing", "quinella": "missing", "wide": "missing"}
    # 頭数不足（dump の行が 1 頭分しかない等）は同着・欠落より優先して別理由にする
    assert pl.exclusion_reasons([1]) == {"win": "field_too_small", "quinella": "field_too_small", "wide": "field_too_small"}
    assert pl.exclusion_reasons([1, 2]) == {"win": None, "quinella": "field_too_small", "wide": "field_too_small"}
    assert pl.exclusion_reasons([1, 2, 3]) == {"win": None, "quinella": None, "wide": "field_too_small"}


def test_race_scores_skips_too_small_fields():
    # 1 頭分しか行の無いレースは NLL=0・一様=0 で単勝の平均を押し下げるので、全券種で母集合外にする
    s = pl.race_scores(pl.combo_probs(np.array([1.0]), 1.0, 1.0), [1])
    assert all(s[k] is None for k in ("win", "quinella", "top2", "wide", "trio"))
    # 3 頭立てのワイド・3連複は的中確率が自明に 1 なので評価しない
    s = pl.race_scores(pl.combo_probs(np.array([0.5, 0.3, 0.2]), 1.0, 1.0), [1, 2, 3])
    assert s["win"] is not None and s["quinella"] is not None and s["wide"] is None and s["trio"] is None
    # 3 着だけ欠落（3 着馬の中止など）
    assert pl.exclusion_reasons([1, 2, None, 4]) == {"win": None, "quinella": None, "wide": "missing"}


def test_wide_pairs_labels_and_brier_literal():
    # 4 頭・一様・λ=1: ワイド各ペアは 3/C(4,2)=0.5。的中は 3 ペア → Brier 和 = 6 × 0.25 = 1.5
    c = pl.combo_probs(np.full(4, 0.25), 1.0, 1.0)
    s = pl.race_scores(c, [1, 2, 3, 4])
    assert s["wide_pairs"]["y"].sum() == 3
    assert s["wide_pairs"]["brier_sum"] == pytest.approx(1.5)
    assert s["wide_pairs"]["n"] == 6
    # 連対の一様予測: 各馬 2/4 → 二値 log-loss 和 = 4·ln 2
    assert s["top2"]["uni_sum"] == pytest.approx(4 * math.log(2))
    assert s["top2"]["ll_sum"] == pytest.approx(4 * math.log(2))


def test_top2_floor_is_counted():
    c = pl.combo_probs(np.array(P5), L2, L3)
    override = np.array([0.0, 0.5, 0.3, 0.3, 0.2])  # 1 着馬（idx0）に連対確率 0
    s = pl.race_scores(c, [1, 2, 3, 4, 5], p_top2=override)
    assert s["floored"] >= 1
    assert math.isfinite(s["top2"]["ll_sum"])


# ---------- 外部確率 TSV ----------


def _write(text):
    d = tempfile.mkdtemp()
    path = os.path.join(d, "probs.tsv")
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    return path


def test_load_probs_ok_with_optional_top2():
    path = _write("race_id\thorse_num\tp_win\tp_top2\nR1\t1\t0.6\t0.8\nR1\t2\t0.4\t0.9\n")
    m = pl.load_probs(path)
    assert m[("R1", 1)] == {"p_win": pytest.approx(0.6), "p_top2": pytest.approx(0.8), "lam": None}
    assert m[("R1", 2)]["p_top2"] == pytest.approx(0.9)
    # p_top2 列が全行空なら「連対は Harville 由来」の系統として読める
    m = pl.load_probs(_write("race_id\thorse_num\tp_win\tp_top2\nR1\t1\t0.6\t\nR1\t2\t0.4\t\n"))
    assert m[("R1", 1)]["p_top2"] is None and m[("R1", 2)]["p_top2"] is None


def test_load_probs_rejects_partial_top2():
    # 一部の行だけ p_top2 があると連対の定義がレースごとに混ざるので拒否する
    with pytest.raises(ValueError, match="一部の行"):
        pl.load_probs(_write("race_id\thorse_num\tp_win\tp_top2\nR1\t1\t0.6\t0.8\nR1\t2\t0.4\t\n"))


def test_load_probs_errors_carry_path_and_line():
    path = _write("race_id\thorse_num\tp_win\nR1\tx\t0.5\n")
    with pytest.raises(ValueError, match=r"probs\.tsv:2 horse_num"):
        pl.load_probs(path)
    path = _write("race_id\thorse_num\tp_win\nR1\t1\n")
    with pytest.raises(ValueError, match=r"probs\.tsv:2 列が足りません"):
        pl.load_probs(path)


def test_load_probs_rejects_bad_input():
    bad = [
        "race_id\thorse_num\n R1\t1\n",  # p_win 欠落
        "race_id\thorse_num\tp_win\nR1\t1\t0.5\nR1\t1\t0.5\n",  # 重複
        "race_id\thorse_num\tp_win\nR1\t1\t-0.1\n",  # 負
        "race_id\thorse_num\tp_win\nR1\t1\tnan\n",  # 非有限
        "race_id\thorse_num\tp_win\tp_top2\nR1\t1\t0.5\t1.5\n",  # 連対確率が 1 超
    ]
    for text in bad:
        with pytest.raises(ValueError):
            pl.load_probs(_write(text))


def test_load_probs_reads_per_race_lambda():
    # 学習した λ を持つ系統（#720）は lam2・lam3 列でレースごとの λ を渡す
    text = "race_id\thorse_num\tp_win\tlam2\tlam3\nR1\t1\t0.6\t0.8\t0.6\nR1\t2\t0.4\t0.8\t0.6\n"
    m = pl.load_probs(_write(text))
    assert m[("R1", 1)]["lam"] == (pytest.approx(0.8), pytest.approx(0.6))
    # 列が無ければ None（系統の λ は --lambda / 既定 1,1）
    assert pl.load_probs(_write("race_id\thorse_num\tp_win\nR1\t1\t1.0\n"))[("R1", 1)]["lam"] is None


def test_load_probs_lambda_range_bounds_are_inclusive():
    lo, hi = pl.TSV_LAM_RANGE
    text = f"race_id\thorse_num\tp_win\tlam2\tlam3\nR1\t1\t1.0\t{lo}\t{hi}\n"
    assert pl.load_probs(_write(text))[("R1", 1)]["lam"] == (lo, hi)


def test_load_probs_rejects_bad_lambda():
    h = "race_id\thorse_num\tp_win\tlam2\tlam3\n"
    bad = [
        ("race_id\thorse_num\tp_win\tlam2\nR1\t1\t1.0\t0.8\n", "両方"),  # 片方の列だけ
        (h + "R1\t1\t0.6\t0.8\t0.6\nR1\t2\t0.4\t\t\n", "一部の行"),  # 一部の行だけ
        (h + "R1\t1\t0.6\t0.8\t0.6\nR1\t2\t0.4\t0.9\t0.6\n", "レース内"),  # レース内で不一致
        (h + "R1\t1\t1.0\t0\t0.6\n", "有限の正の数"),  # 0 は 0**0=1 になる
        (h + "R1\t1\t1.0\tinf\t0.6\n", "有限の正の数"),
        (h + "R1\t1\t1.0\tx\t0.6\n", "数値"),
        (h + "R1\t1\t1.0\t1e6\t0.6\n", "値域"),  # 確率が 0 に潰れる
        (h + "R1\t1\t1.0\t0.8\t0.01\n", "値域"),
    ]
    for text, msg in bad:
        with pytest.raises(ValueError, match=msg):
            pl.load_probs(_write(text))


# ---------- 窓 ----------


def test_validate_windows4():
    assert pl.validate_windows4("2025-07-01", "2025-12-31", "2026-01-01", "2026-08-31") is None
    assert pl.validate_windows4("2025-07-01", "2026-01-15", "2026-01-01", "2026-08-31")  # 重複
    assert pl.validate_windows4("2025-07-01", "2026-01-01", "2026-01-01", "2026-08-31")  # 境界日の共有
    assert pl.validate_windows4("2025-12-31", "2025-07-01", "2026-01-01", "2026-08-31")  # 逆転
    assert pl.validate_windows4("2025/07/01", "2025-12-31", "2026-01-01", "2026-08-31")  # 形式
    # dev が test より後にあるだけ（重なりなし）は「前に置く」旨のメッセージ
    assert "より前に" in pl.validate_windows4("2026-09-01", "2026-12-31", "2026-01-01", "2026-08-31")
    # dev を凍結 eval 窓に重ねると記録なしで eval 窓を覗けるので拒否する
    assert "凍結" in pl.validate_windows4("2026-01-01", "2026-08-31", "2026-09-01", "2026-12-31")
    # basic 形式は fromisoformat（3.11+）を通るが、文字列比較で窓が黙って空になるので拒否する
    # （test-to を basic 形式にすると大小比較の検査はすり抜けるので、形式検査だけが頼り）
    assert pl.validate_windows4("2025-07-01", "2025-12-31", "2026-01-01", "20260831")


def test_in_window():
    assert pl.in_window("2025-07-01", "2025-07-01", "2025-12-31")
    assert pl.in_window("2025-12-31", "2025-07-01", "2025-12-31")
    assert not pl.in_window("2025-06-30", "2025-07-01", "2025-12-31")
    assert not pl.in_window("2026-01-01", "2025-07-01", "2025-12-31")


# ---------- 系統の母集合揃え ----------


def _rows(race_id, fins, pure):
    return [
        {"race_id": race_id, "date": "2025-08-01", "horse_num": i + 1, "finishing_position": f,
         "model_win_pure": p, "win_odds": None}
        for i, (f, p) in enumerate(zip(fins, pure))
    ]


def test_system_probs_requires_full_coverage():
    rows = _rows("R1", [1, 2, 3], [0.5, 0.3, 0.2])
    full = {("R1", 1): {"p_win": 0.5, "p_top2": None}, ("R1", 2): {"p_win": 0.3, "p_top2": None},
            ("R1", 3): {"p_win": 0.2, "p_top2": None}}
    p, t2, lam = pl.system_probs(rows, full)
    assert np.allclose(p, [0.5, 0.3, 0.2]) and t2 is None and lam is None
    with_lam = {k: {**v, "lam": (0.8, 0.6)} for k, v in full.items()}
    assert pl.system_probs(rows, with_lam)[2] == (0.8, 0.6)
    partial = {k: v for k, v in full.items() if k != ("R1", 3)}
    assert pl.system_probs(rows, partial) == "horse_partial"
    assert pl.system_probs(rows, {}) == "race_absent"
    rows[1]["model_win_pure"] = None
    assert pl.system_probs(rows, None) == "horse_partial"


def test_evaluate_counts_every_missing_system_and_bet_exclusions():
    races = OrderedDict([
        ("R1", _rows("R1", [1, 2, 3, 4], [0.4, 0.3, 0.2, 0.1])),
        ("R2", _rows("R2", [1, 2, 3, 3], [0.1, 0.2, 0.3, 0.4])),  # 3 着同着
        ("R3", _rows("R3", [None, None, None, None], [0.25] * 4)),  # 結果未取込
        ("R4", _rows("R4", [1, 2, 3, 4], [0.25] * 4)),
    ])
    a = {(r, h): {"p_win": 0.25, "p_top2": None} for r in ("R1", "R2", "R3") for h in range(1, 5)}
    b = {(r, h): {"p_win": 0.5, "p_top2": None} for r in ("R1", "R2", "R3") for h in range(1, 5)}  # 和 2.0
    systems = OrderedDict([
        ("pure", pl.System("pure", None)),
        ("a", pl.System("a", a)),
        ("b", pl.System("b", b)),
    ])
    res = pl.evaluate(races, systems)
    # R4 は a・b の両方が欠くので、両方に計上する（最初の系統で打ち切らない）
    assert res["excluded"]["race_absent:a"] == 1 and res["excluded"]["race_absent:b"] == 1
    assert res["n_races"] == 3
    assert res["bet_excluded"][("wide", "dead_heat")] == 1
    assert res["bet_excluded"][("win", "missing")] == 1
    assert res["bet_excluded"][("quinella", "missing")] == 1
    assert res["bet_excluded"][("wide", "missing")] == 1
    assert res["bet_excluded"][("win", "dead_heat")] == 0
    # p_win の和が 1 から外れた外部系統は（正規化して評価しつつ）警告に数える
    assert res["warnings"]["p_win_sum_off:b"] == 3 and res["warnings"]["p_win_sum_off:a"] == 0


def test_evaluate_warns_on_suspicious_inputs():
    rows = _rows("R1", [1, 2, 3, 4], [0.4, 0.3, 0.2, 0.1])
    rows[3]["horse_num"] = 6  # 行数 4 < 馬番の最大 6（取消・除外か行の欠け）
    races = OrderedDict([("R1", rows)])
    ext = {("R1", h): {"p_win": 0.25, "p_top2": 0.25} for h in (1, 2, 3, 6)}  # 連対の和 1.0（定義違い）
    ext[("R1", 9)] = {"p_win": 0.1, "p_top2": 0.1}  # dump に無い馬
    systems = OrderedDict([("pure", pl.System("pure", None)), ("ext", pl.System("ext", ext))])
    res = pl.evaluate(races, systems)
    assert res["warnings"]["tsv_extra_horse:ext"] == 1
    assert res["warnings"]["rows_lt_max_horse_num"] == 1
    assert res["warnings"]["p_top2_sum_off:ext"] == 1
    assert res["n_races"] == 1


def test_evaluate_applies_system_lambda_and_direct_top2():
    fins = [2, 1, 3, 4, 5]
    races = OrderedDict([("R1", _rows("R1", fins, P5))])
    top2 = [0.7, 0.6, 0.3, 0.2, 0.2]
    ext = {("R1", h + 1): {"p_win": P5[h], "p_top2": top2[h]} for h in range(5)}
    systems = OrderedDict([
        ("pure", pl.System("pure", None, 1.0, 1.0)),
        ("ext", pl.System("ext", ext, L2, L3)),
    ])
    res = pl.evaluate(races, systems)
    ext_s = res["scores"]["ext"][0]
    pure_s = res["scores"]["pure"][0]
    # 系統ごとの λ が組合せ確率に届いている（同じ p_win でも λ が違えば馬連 NLL が違う）
    assert ext_s["quinella"][0] == pytest.approx(-math.log(pl.combo_probs(np.array(P5), L2, L3).quinella[0, 1]))
    assert pure_s["quinella"][0] == pytest.approx(-math.log(pl.combo_probs(np.array(P5), 1.0, 1.0).quinella[0, 1]))
    assert ext_s["quinella"][0] != pytest.approx(pure_s["quinella"][0])
    # 直接出力の連対確率が使われている
    assert np.allclose(ext_s["top2"]["p"], top2)


def test_evaluate_uses_per_race_lambda_from_tsv():
    # レースごとの λ（TSV）があれば系統の λ より優先し、レースごとに違う λ で組合せ確率を作る
    races = OrderedDict([("R1", _rows("R1", [2, 1, 3, 4, 5], P5)), ("R2", _rows("R2", [2, 1, 3, 4, 5], P5))])
    lam_of = {"R1": (0.6, 0.5), "R2": (1.4, 1.2)}
    ext = {(r, h + 1): {"p_win": P5[h], "p_top2": None, "lam": lam_of[r]} for r in lam_of for h in range(5)}
    systems = OrderedDict([("pure", pl.System("pure", None)), ("ext", pl.System("ext", ext))])
    res = pl.evaluate(races, systems)
    for s_, rid in zip(res["scores"]["ext"], res["race_ids"]):
        q = pl.combo_probs(np.array(P5), *lam_of[rid]).quinella[0, 1]
        assert s_["quinella"][0] == pytest.approx(-math.log(q))
    assert res["scores"]["ext"][0]["wide"][0] != pytest.approx(res["scores"]["ext"][1]["wide"][0])


def test_evaluate_market_applies_lambda():
    rows = _rows("R1", [2, 1, 3, 4, 5], P5)
    for r, o in zip(rows, [2.0, 3.5, 6.0, 8.0, 12.0]):
        r["win_odds"] = o
    races = OrderedDict([("R1", rows)])
    m1 = pl.evaluate_market(races, 1.0, 1.0)[0]
    m2 = pl.evaluate_market(races, L2, L3)[0]
    assert m1["quinella"][0] != pytest.approx(m2["quinella"][0])


def test_fidelity_population_matches_prob_eval_rule():
    ok = _rows("R1", [1, 2, 3], [0.5, 0.3, 0.2])
    for r, o in zip(ok, [2.0, 3.0, 6.0]):
        r["win_odds"] = o
    no_odds = _rows("R2", [1, 2, 3], [0.9, 0.05, 0.05])  # 市場オッズなし → 母集合外
    no_pure = _rows("R3", [1, 2, 3], [0.4, None, 0.2])
    for r, o in zip(no_pure, [2.0, 3.0, 6.0]):
        r["win_odds"] = o
    brier, n = pl.fidelity_pure_win_brier(OrderedDict([("R1", ok), ("R2", no_odds), ("R3", no_pure)]))
    assert n == 1
    assert brier == pytest.approx(((0.5 - 1) ** 2 + 0.3 ** 2 + 0.2 ** 2) / 3)


def test_corp_summary_flags_miscalibration():
    # 連対確率 0.9 と言い続けて実際は 2/10 → 帯外点が出る
    c = pl.combo_probs(np.full(10, 0.1), 1.0, 1.0)
    scores = [pl.race_scores(c, list(range(1, 11)), p_top2=np.full(10, 0.9)) for _ in range(30)]
    d = pl.corp_summary(scores, "top2", n_boot=50, seed=1)
    assert d["outside"] > 0
    assert d["n"] == 300


def test_evaluate_aligns_races_and_counts_exclusions():
    races = OrderedDict([
        ("R1", _rows("R1", [1, 2, 3, 4], [0.4, 0.3, 0.2, 0.1])),
        ("R2", _rows("R2", [2, 1, 3, 4], [0.1, 0.2, 0.3, 0.4])),
    ])
    ext = {("R1", h): {"p_win": 0.25, "p_top2": None} for h in range(1, 5)}  # R2 を欠く
    systems = OrderedDict([
        ("pure", pl.System("pure", None, 1.0, 1.0)),
        ("ext", pl.System("ext", ext, 1.0, 1.0)),
    ])
    res = pl.evaluate(races, systems)
    assert res["n_races"] == 1
    assert res["excluded"]["race_absent:ext"] == 1
    assert set(res["scores"]) == {"pure", "ext"}
    assert len(res["scores"]["pure"]) == 1


# ---------- 集計・CI ----------


def test_judge_delta_direction():
    # NLL・Brier は小さいほど良い、R² は大きいほど良い
    assert pl.judge_delta("quinella_nll", -0.2, -0.1) == " ✅"
    assert pl.judge_delta("quinella_nll", 0.1, 0.2) == " ❌"
    assert pl.judge_delta("quinella_nll", -0.1, 0.1) == ""
    assert pl.judge_delta("quinella_r2", 0.01, 0.02) == " ✅"
    assert pl.judge_delta("quinella_r2", -0.02, -0.01) == " ❌"
    assert pl.judge_delta("top2_brier", -0.02, -0.01) == " ✅"


def _varied_scores():
    c = pl.combo_probs(np.array(P5), L2, L3)
    orders = [[1, 2, 3, 4, 5], [5, 4, 3, 2, 1], [2, 5, 1, 3, 4], [3, 1, 4, 5, 2], [4, 3, 5, 1, 2]]
    return [pl.race_scores(c, o) for o in orders]


def test_paired_ci_uses_the_same_resample():
    # B の馬連 NLL を全レース一律 +0.3 にすると、対応あり（同じ再標本）なら差の CI はちょうど −0.3。
    # 別々に再標本すると幅が出るので、この性質は A/B に同じ重みを掛けていることの検出になる。
    a = _varied_scores()
    assert len({round(s["quinella"][0], 6) for s in a}) > 1  # レース間でばらついていること
    b = []
    for s in a:
        t = dict(s)
        t["quinella"] = (s["quinella"][0] + 0.3, s["quinella"][1])
        b.append(t)
    lo, hi = pl.boot_ci(a, b, "quinella_nll", n_boot=200, seed=3)
    assert lo == pytest.approx(-0.3) and hi == pytest.approx(-0.3)
    lo1, hi1 = pl.boot_ci(a, None, "quinella_nll", n_boot=200, seed=3)
    assert hi1 - lo1 > 0.01


def test_aggregate_denominators_per_bet_type():
    c = pl.combo_probs(np.array(P5), L2, L3)
    full = pl.race_scores(c, [1, 2, 3, 4, 5])
    dh3 = pl.race_scores(c, [1, 2, 3, 3, 5])  # 3 着同着: 馬連・連対はあり、ワイドは無し
    agg = pl.aggregate([full, dh3])
    assert agg["quinella_n"] == 2 and agg["wide_n"] == 1
    assert agg["wide_brier"] == pytest.approx(full["wide_pairs"]["brier_sum"] / 10)
    assert agg["quinella_brier"] == pytest.approx(
        (full["quinella_pairs"]["brier_sum"] + dh3["quinella_pairs"]["brier_sum"]) / 20
    )
    assert agg["top2_brier"] == pytest.approx((full["top2"]["brier_sum"] + dh3["top2"]["brier_sum"]) / 10)
    assert agg["top2_r2"] == pytest.approx(
        1 - (full["top2"]["ll_sum"] + dh3["top2"]["ll_sum"]) / (full["top2"]["uni_sum"] + dh3["top2"]["uni_sum"])
    )


def test_aggregate_and_paired_ci():
    c = pl.combo_probs(np.array(P5), L2, L3)
    scores = [pl.race_scores(c, [1, 2, 3, 4, 5]), pl.race_scores(c, [2, 1, 3, 5, 4])]
    agg = pl.aggregate(scores)
    assert agg["quinella_nll"] == pytest.approx(np.mean([s["quinella"][0] for s in scores]))
    r2 = 1 - sum(s["quinella"][0] for s in scores) / sum(s["quinella"][1] for s in scores)
    assert agg["quinella_r2"] == pytest.approx(r2)
    lo, hi = pl.boot_ci(scores, scores, "quinella_nll", n_boot=50, seed=1)
    assert lo == pytest.approx(0.0) and hi == pytest.approx(0.0)
    lo, hi = pl.boot_ci(scores, None, "quinella_nll", n_boot=50, seed=1)
    assert lo <= agg["quinella_nll"] <= hi


# ---------- レポート ----------


def _race_rows(race_id, date, fins, pure):
    return [
        {"race_id": race_id, "date": date, "horse_num": i + 1, "finishing_position": f,
         "model_win_pure": p, "win_odds": 2.0 + i}
        for i, (f, p) in enumerate(zip(fins, pure))
    ]


def _table_widths(lines):
    return [ln.count("|") for ln in lines if ln.startswith("|")]


def test_report_tables_are_well_formed():
    # 系統 1 本でも 2 本でも、表の各行の列数が見出し・区切りと一致する（markdown が崩れない）。
    races = OrderedDict(
        (f"R{k}", _race_rows(f"R{k}", "2025-08-01", [1, 2, 3, 4, 5], [0.4, 0.25, 0.15, 0.12, 0.08]))
        for k in range(4)
    )
    ext = {(f"R{k}", h): {"p_win": 0.2, "p_top2": None} for k in range(4) for h in range(1, 6)}
    for systems in (
        OrderedDict([("pure", pl.System("pure", None))]),
        OrderedDict([("pure", pl.System("pure", None)), ("ext", pl.System("ext", ext))]),
    ):
        lines, res = pl.report_window("dev", races, systems, (1.0, 1.0), n_boot=20, seed=1)
        assert res["n_races"] == 4
        metric_table = [ln for ln in lines if ln.startswith("|")][: 2 + len(pl.METRICS)]
        widths = _table_widths(metric_table)
        assert len(set(widths)) == 1, widths
        assert widths[0] == 2 + len(systems) + (len(systems) - 1)


# ---------- prob_eval との契約（import して使う前提を固定） ----------


def test_prob_eval_contract():
    import prob_eval as pe

    assert 0 < pe._PROB_FLOOR <= 1e-6
    # 着順なしは不的中（0）として扱う
    assert pe._label({"finishing_position": None}, 2) == 0.0
    assert pe._label({"finishing_position": 2}, 2) == 1.0
    # オッズ欠落・1.0 未満のレースは市場確率なし
    assert pe.market_probs([{"win_odds": 2.0}, {"win_odds": None}]) is None
    assert pe.market_probs([{"win_odds": 2.0}, {"win_odds": 0.9}]) is None
    q = pe.market_probs([{"win_odds": 2.0}, {"win_odds": 4.0}])
    assert q == [pytest.approx(2 / 3), pytest.approx(1 / 3)]


# ---------- CLI（main） ----------

DUMP_HEADER = [
    "race_id", "date", "horse_num", "model_win", "model_place", "model_show",
    "model_win_pure", "model_place_pure", "model_show_pure", "finishing_position", "win_odds", "popularity",
]


def _write_dump(races):
    """races: [(race_id, date, [(horse_num, pure, fin, odds)])] を 12 列の dump にする。"""
    lines = ["\t".join(DUMP_HEADER)]
    for rid, d, horses in races:
        for h, p, f, o in horses:
            lines.append("\t".join(map(str, [rid, d, h, p, p, p, p, p, p, f, o, h])))
    return _write("\n".join(lines) + "\n")


def _cli_fixture():
    horses = [(1, 0.4, 1, 2.0), (2, 0.3, 2, 3.0), (3, 0.2, 3, 5.0), (4, 0.1, 4, 9.0)]
    dump = _write_dump([
        ("R0", "2025-06-30", horses),  # dev 窓の外
        ("R1", "2025-08-01", horses),
        ("R2", "2025-09-01", horses),
        ("R3", "2026-02-01", horses),  # test 窓
    ])
    ext = _write("race_id\thorse_num\tp_win\n" + "".join(
        f"{r}\t{h}\t0.25\n" for r in ("R0", "R1", "R2", "R3") for h in range(1, 5)
    ))
    return dump, ext


def _run(monkeypatch, capsys, argv, dirty=False):
    monkeypatch.setattr(pl, "code_version", lambda: ("abc1234", dirty))
    rc = pl.main(argv)
    return rc, capsys.readouterr().out


def test_main_dev_window_lambda_and_sources(monkeypatch, capsys):
    dump, ext = _cli_fixture()
    rc, out = _run(monkeypatch, capsys, [dump, "--system", f"ext={ext}", "--lambda", "ext=0.5,0.7", "--bootstrap", "20"])
    assert rc == 0
    assert "対象レース 2（窓内 2" in out  # 窓外の R0・test 窓の R3 は入らない
    assert "ext（λ=0.5,0.7・`probs.tsv` sha256 " in out
    assert "pure（λ=1.0,1.0）" in out
    assert "### test" not in out
    assert "券種別の除外" in out


def test_main_per_race_lambda_system(monkeypatch, capsys):
    dump, _ = _cli_fixture()
    lam_tsv = _write("race_id\thorse_num\tp_win\tlam2\tlam3\n" + "".join(
        f"{r}\t{h}\t0.25\t0.8\t0.6\n" for r in ("R0", "R1", "R2", "R3") for h in range(1, 5)
    ))
    rc, out = _run(monkeypatch, capsys, [dump, "--system", f"pl={lam_tsv}", "--bootstrap", "20"])
    assert rc == 0 and "pl（λ=TSV のレース別・" in out
    # TSV が λ を持つ系統に --lambda を重ねると、どちらが効いたか曖昧になるので拒否する
    with pytest.raises(SystemExit):
        _run(monkeypatch, capsys, [dump, "--system", f"pl={lam_tsv}", "--lambda", "pl=0.9,0.8"])
    assert "TSV" in capsys.readouterr().err


def test_main_rejects_bad_arguments(monkeypatch, capsys):
    dump, ext = _cli_fixture()
    weird_dir = tempfile.mkdtemp()
    weird = os.path.join(weird_dir, "a|b.tsv")
    with open(weird, "w", encoding="utf-8") as f:
        f.write(open(ext, encoding="utf-8").read())
    bad = [
        ([dump, "--windows", "test"], "必ず記録"),  # test 窓は記録必須
        ([dump, "--system", f"a={ext}", "--system", f"a={ext}"], "重複"),  # 系統名の重複
        ([dump, "--system", f"a={ext}", "--lambda", "b=0.9,0.8"], "存在しない系統名"),
        ([dump, "--lambda", "pure=nan,0.8"], "有限の正の数"),
        ([dump, "--lambda", "pure=0,0.8"], "有限の正の数"),
        ([dump, "--system", f"a|b={ext}"], "英数字"),  # 表を壊す系統名
        ([dump, "--ledger", _write(""), "--label", "x\n## 偽の節"], "1 行"),  # 改行入りの見出し
        ([dump, "--system", f"w={weird}"], "ファイル名"),  # 表を壊すファイル名
        ([dump, "--dev-from", "2026-01-01", "--dev-to", "2026-08-31", "--test-from", "2026-09-01",
          "--test-to", "2026-12-31"], "凍結"),  # dev で eval 窓を覗く
    ]
    for argv, msg in bad:
        with pytest.raises(SystemExit):
            _run(monkeypatch, capsys, argv)
        assert msg in capsys.readouterr().err, (argv, msg)


def test_main_fidelity_note_per_window(monkeypatch, capsys):
    dump, _ = _cli_fixture()
    ledger = _write("# ledger\n")
    _run(monkeypatch, capsys, [dump, "--windows", "both", "--ledger", ledger, "--label", "v", "--bootstrap", "20"])
    text = open(ledger, encoding="utf-8").read()
    dev, test = text.split("### test")
    assert "参考値。prob_eval に同じ窓は無い" in dev
    assert "prob_eval の eval 窓の win×pure と一致すること" in test


def test_main_ledger_refuses_empty_window(monkeypatch, capsys):
    dump, _ = _cli_fixture()
    ledger = _write("# ledger\n")
    with pytest.raises(SystemExit):
        _run(monkeypatch, capsys, [dump, "--windows", "both", "--test-from", "2026-01-01", "--test-to", "2026-01-31",
                                   "--ledger", ledger, "--label", "v", "--bootstrap", "20"])
    assert "0 件" in capsys.readouterr().err
    assert open(ledger, encoding="utf-8").read() == "# ledger\n"


def test_code_version_fails_closed_and_sees_untracked_py(monkeypatch):
    def boom(*a):
        raise subprocess.CalledProcessError(1, "git")

    monkeypatch.setattr(pl, "_git_run", boom)
    assert pl.code_version() == ("unknown", True)

    def fake(status):
        def run(*args):
            if args[0] == "rev-parse":
                return "abc1234" if args[1] == "--short" else "/repo"
            return status
        return run

    monkeypatch.setattr(pl, "_git_run", fake(""))
    assert pl.code_version() == ("abc1234", False)
    monkeypatch.setattr(pl, "_git_run", fake("?? scripts/predict-check/new_model.py"))
    assert pl.code_version()[1] is True
    monkeypatch.setattr(pl, "_git_run", fake("?? scripts/predict-check/notes.txt"))
    assert pl.code_version()[1] is False
    monkeypatch.setattr(pl, "_git_run", fake(" M scripts/predict-check/prob_eval.py"))
    assert pl.code_version()[1] is True


def test_main_ledger_requires_clean_code_and_appends(monkeypatch, capsys):
    dump, ext = _cli_fixture()
    ledger = _write("# ledger\n")
    with pytest.raises(SystemExit):
        _run(monkeypatch, capsys, [dump, "--windows", "both", "--ledger", ledger, "--label", "v"], dirty=True)
    rc, _ = _run(monkeypatch, capsys, [dump, "--windows", "both", "--ledger", ledger, "--label", "v1", "--bootstrap", "20"])
    assert rc == 0
    text = open(ledger, encoding="utf-8").read()
    assert text.startswith("# ledger\n") and "## v1" in text
    assert "git abc1234" in text and "-dirty" not in text
    assert "### dev" in text and "### test" in text


# ---------- ledger ----------


def test_append_ledger_writes_section():
    d = tempfile.mkdtemp()
    path = os.path.join(d, "ledger.md")
    pl.append_ledger(path, "baseline v0", ["| a | b |", "|---|---|"])
    pl.append_ledger(path, "v1", ["x"])
    text = open(path, encoding="utf-8").read()
    assert "## baseline v0" in text and "## v1" in text
    assert text.index("baseline v0") < text.index("v1")


if __name__ == "__main__":
    sys.exit(pytest.main([__file__, "-q"]))
