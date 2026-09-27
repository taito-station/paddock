"""prob_ledger.py のユニットテスト（#719）。

対象: 割引 Harville の組合せ確率（馬連・連対・ワイド・3連複）を総当たりの独立実装と照合 /
和の恒等式 / λ=1 での umaren_backtest との一致 / λ≠1 での harville_lambda_fit.stage_loglik との一致 /
券種別の母集合（同着・着順欠落）/ レース単位のスコアと一様比 / floor / 外部確率 TSV の読み込み /
4 端点の窓検証 / 系統の母集合揃え / ブートストラップ CI / ledger 追記。
"""

import math
import os
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


# ---------- 外部確率 TSV ----------


def _write(text):
    d = tempfile.mkdtemp()
    path = os.path.join(d, "probs.tsv")
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    return path


def test_load_probs_ok_with_optional_top2():
    path = _write("race_id\thorse_num\tp_win\tp_top2\nR1\t1\t0.6\t0.8\nR1\t2\t0.4\t\n")
    m = pl.load_probs(path)
    assert m[("R1", 1)] == {"p_win": pytest.approx(0.6), "p_top2": pytest.approx(0.8)}
    assert m[("R1", 2)]["p_top2"] is None


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


# ---------- 窓 ----------


def test_validate_windows4():
    assert pl.validate_windows4("2025-07-01", "2025-12-31", "2026-01-01", "2026-08-31") is None
    assert pl.validate_windows4("2025-07-01", "2026-01-15", "2026-01-01", "2026-08-31")  # 重複
    assert pl.validate_windows4("2025-07-01", "2026-01-01", "2026-01-01", "2026-08-31")  # 境界日の共有
    assert pl.validate_windows4("2025-12-31", "2025-07-01", "2026-01-01", "2026-08-31")  # 逆転
    assert pl.validate_windows4("2025/07/01", "2025-12-31", "2026-01-01", "2026-08-31")  # 形式


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
    p, t2 = pl.system_probs(rows, full)
    assert np.allclose(p, [0.5, 0.3, 0.2]) and t2 is None
    partial = {k: v for k, v in full.items() if k != ("R1", 3)}
    assert pl.system_probs(rows, partial) is None


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
    assert res["excluded"]["system_missing:ext"] == 1
    assert set(res["scores"]) == {"pure", "ext"}
    assert len(res["scores"]["pure"]) == 1


# ---------- 集計・CI ----------


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
        lines = pl.report_window("dev", races, systems, (1.0, 1.0), n_boot=20, seed=1)
        metric_table = [ln for ln in lines if ln.startswith("|")][: 2 + len(pl.METRICS)]
        widths = _table_widths(metric_table)
        assert len(set(widths)) == 1, widths
        assert widths[0] == 2 + len(systems) + (len(systems) - 1)


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
