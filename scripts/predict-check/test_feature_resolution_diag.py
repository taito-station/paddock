#!/usr/bin/env python3
"""feature_resolution_diag の鏡映関数・PAVA の単体テスト。

`python3 -m pytest scripts/predict-check/test_feature_resolution_diag.py` または
`python3 scripts/predict-check/test_feature_resolution_diag.py`（簡易 runner）で実行。
"""

import feature_resolution_diag as d


def test_shrink_rate_pulls_small_sample_to_prior():
    # k=0 で prior、k≫m で生レートに収束。
    assert abs(d.shrink_rate(1.0, 0, 0.1) - 0.1) < 1e-12
    assert abs(d.shrink_rate(0.5, 1000, 0.1) - 0.5) < 1e-2
    # 単調補間: k 増で rate 側へ。
    assert d.shrink_rate(1.0, 5, 0.1) < d.shrink_rate(1.0, 50, 0.1)


def test_normalize_to_sum_targets_and_clamps():
    out = d.normalize_to_sum([1.0, 3.0], 1.0)
    assert abs(sum(out) - 1.0) < 1e-12
    assert abs(out[1] - 0.75) < 1e-12
    # 全 0 は均等フォールバック。
    assert d.normalize_to_sum([0.0, 0.0, 0.0], 3.0) == [1.0, 1.0, 1.0]


def test_win_power_sharpens_and_renormalizes():
    out = d.win_power([0.5, 0.3, 0.2], 1.25)
    assert abs(sum(out) - 1.0) < 1e-9
    # γ>1 は本命を相対強調（最大値のシェアが上がる）。
    assert out[0] > 0.5  # 0.5 のシェアが上がる
    # γ=1 は no-op。
    assert d.win_power([0.5, 0.5], 1.0) == [0.5, 0.5]


def test_score_power_noop_on_one():
    assert d.score_power([0.3, 0.7], 1.0) == [0.3, 0.7]  # γ=1 は厳密 no-op（同一リスト）
    out = d.score_power([0.2, 0.4], 2.0)
    assert all(abs(a - b) < 1e-12 for a, b in zip(out, [0.04, 0.16]))


def _row(course=None, surface=None, form=None):
    """36 列の行を最小構成で作る（テスト用）。course/surface は (win,place,show,starts)。"""
    cells = [""] * d.N_COLS
    cells[0], cells[1], cells[2] = "R1", "2026-01-01", "1"
    if course:
        cells[3:7] = [str(x) for x in course]
    if surface:
        cells[7:11] = [str(x) for x in surface]
    if form is not None:
        cells[27] = str(form)
    cells[30] = cells[31] = cells[32] = "0"  # model_* ダミー
    cells[33] = "1"
    return cells


def test_raw_score_weighted_average_with_shrinkage():
    # course_gate(w2.0, win=0.3, k=10) と horse_surface(w1.0, win=0.6, k=10) のみ。
    r = _row(course=(0.3, 0.4, 0.5, 10), surface=(0.6, 0.7, 0.8, 10))
    sc = d.shrink_rate(0.3, 10, d.PRIOR["win"])
    ss = d.shrink_rate(0.6, 10, d.PRIOR["win"])
    expect = (2.0 * sc + 1.0 * ss) / 3.0
    assert abs(d.raw_score(r, "win") - expect) < 1e-12


def test_raw_score_scalar_form_same_for_all_sel():
    # recent_form(スカラー, w0.25) は win/place/show に同値で寄与。surface のみ + form。
    r = _row(surface=(0.6, 0.6, 0.6, 10), form=0.8)
    for sel in ("win", "place", "show"):
        ss = d.shrink_rate(0.6, 10, d.PRIOR[sel])
        expect = (1.0 * ss + 0.25 * 0.8) / 1.25
        assert abs(d.raw_score(r, sel) - expect) < 1e-12


def test_raw_score_drop_excludes_factor():
    r = _row(course=(0.3, 0.4, 0.5, 10), surface=(0.6, 0.7, 0.8, 10))
    # course を drop すると surface のみ（縮約後の生値）。
    ss = d.shrink_rate(0.6, 10, d.PRIOR["win"])
    assert abs(d.raw_score(r, "win", drop="course_gate") - ss) < 1e-12


def test_raw_score_all_missing_returns_zero():
    assert d.raw_score(_row(), "win") == 0.0


def test_raw_score_weights_override():
    # #272 改善①: weights override。None は既定と一致、重み 0 は drop と同値、変更で値が変わる。
    r = _row(course=(0.3, 0.4, 0.5, 10), surface=(0.6, 0.7, 0.8, 10))
    assert d.raw_score(r, "win") == d.raw_score(r, "win", weights=None)
    sc0 = d.raw_score(r, "win", weights={"course_gate": 0.0})
    assert abs(sc0 - d.raw_score(r, "win", drop="course_gate")) < 1e-12
    assert d.raw_score(r, "win", weights={"course_gate": 5.0}) != d.raw_score(r, "win")


def test_race_probs_monotone_and_normalized():
    # race_probs（win/place/show 合成）は win≤place≤show の単調性と win 合計≒1.0 を保つ。
    rows = [
        _row(course=(0.3, 0.4, 0.5, 10), surface=(0.6, 0.7, 0.8, 12)),
        _row(course=(0.1, 0.2, 0.3, 8), surface=(0.2, 0.3, 0.4, 9)),
        _row(surface=(0.5, 0.6, 0.7, 5), form=0.7),
    ]
    win_p, place_p, show_p = d.race_probs(rows)
    assert abs(sum(win_p) - 1.0) < 1e-9, "win 合計 ≒ 1.0"
    for i in range(len(rows)):
        assert 0.0 <= win_p[i] <= place_p[i] <= show_p[i] <= 1.0, f"単調性/範囲: row {i}"
    # drop で素性を外しても単調性は保たれる。
    w2, p2, s2 = d.race_probs(rows, drop="course_gate")
    for i in range(len(rows)):
        assert w2[i] <= p2[i] <= s2[i]


def test_pava_monotone_and_fits():
    # 非単調入力 [3,1,2] を単調化（PAVA で平均ブロック）。
    thr, fit = d.pava_fit([1.0, 2.0, 3.0], [3.0, 1.0, 2.0])
    assert all(fit[i] <= fit[i + 1] + 1e-12 for i in range(len(fit) - 1)), "単調増加"
    # 完全単調なデータはそのまま。
    thr2, fit2 = d.pava_fit([1, 2, 3], [0.1, 0.5, 0.9])
    assert all(abs(a - b) < 1e-12 for a, b in zip(fit2, [0.1, 0.5, 0.9]))


def test_pava_apply_steps():
    thr, fit = d.pava_fit([0.1, 0.5, 0.9], [0.0, 0.4, 1.0])
    out = [d.pava_apply(thr, fit, x) for x in (0.05, 0.5, 0.95)]
    assert out[0] <= out[1] <= out[2]


def test_auc_perfect_and_random():
    # 完全分離 → AUC=1。
    assert abs(d.auc([0.1, 0.2, 0.9, 0.95], [0, 0, 1, 1]) - 1.0) < 1e-9
    # 反転 → AUC=0。
    assert abs(d.auc([0.9, 0.95, 0.1, 0.2], [0, 0, 1, 1]) - 0.0) < 1e-9


def test_brier_and_logloss_basic():
    assert abs(d.brier([1.0, 0.0], [1, 0]) - 0.0) < 1e-12
    assert d.logloss([0.5, 0.5], [1, 0]) > 0.0


if __name__ == "__main__":
    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]
    failed = 0
    for fn in fns:
        try:
            fn()
            print(f"ok   {fn.__name__}")
        except Exception as e:  # noqa: BLE001
            failed += 1
            print(f"FAIL {fn.__name__}: {e}")
    print(f"\n{len(fns) - failed}/{len(fns)} passed")
    raise SystemExit(1 if failed else 0)
