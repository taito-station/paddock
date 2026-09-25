#!/usr/bin/env python3
"""zure_sign_probe.py のユニットテスト（自走式・stdlib のみ・CI predict-check 対象）。

中核は (1) Δ の符号規約（Δ>0 = オッズ低下）と事前コミット判定 judge の対応、
(2) t_prev/t_last 選択の JST 日付・lag・最近傍規則、(3) ロジスティックが合成データの
係数を回収するか、(4) netkeiba 結果ページの確定オッズパーサ。
"""

import calendar
import math
import random
import sys

import zure_sign_probe as zp


def _jst_epoch(y, mo, d, h, mi, se=0):
    """JST の壁時計 → UTC epoch 秒（テストは JST で書くと読みやすい）。"""
    return calendar.timegm((y, mo, d, h, mi, se, 0, 0, 0)) - zp.JST_OFFSET_SEC


def test_jst_date_min():
    # JST 2026-08-23 00:30 は UTC では前日 15:30 — JST 側の日付・分が返ること。
    e = _jst_epoch(2026, 8, 23, 0, 30)
    d, mn = zp.jst_date_min(e)
    assert d == "2026-08-23"
    assert abs(mn - 30) < 1e-9


def test_hhmm_to_min():
    assert zp.hhmm_to_min("15:10") == 910
    assert zp.hhmm_to_min("9:05") == 545
    assert zp.hhmm_to_min(None) is None
    assert zp.hhmm_to_min("") is None
    assert zp.hhmm_to_min("25時") is None


def test_select_points_filters_and_nearest():
    date = "2026-08-23"
    post = zp.hhmm_to_min("15:10")
    t = lambda h, mi: _jst_epoch(2026, 8, 23, h, mi)  # noqa: E731
    ts = sorted([
        _jst_epoch(2026, 8, 24, 9, 0),   # 翌日再スクレイプ（#568）→ 除外
        t(15, 20),                        # 発走後 → 除外
        t(13, 0), t(14, 40), t(14, 52), t(15, 5),
    ])
    prev, last = zp.select_points(ts, date, post)
    assert last == t(15, 5)   # 発走前の最後
    # 10 分以上前の最近傍 = 14:52（差 13 分）。14:40 ではない（最近傍）、15:05-15:05=0 は不可。
    assert prev == t(14, 52)


def test_select_points_failure_reasons():
    date = "2026-08-23"
    post = zp.hhmm_to_min("15:10")
    # 発走前 snapshot ゼロ
    r = zp.select_points([_jst_epoch(2026, 8, 23, 15, 30)], date, post)
    assert r == (None, "no_prerace_snapshot")
    # 1 時点のみ → t_prev なし
    r = zp.select_points([_jst_epoch(2026, 8, 23, 15, 0)], date, post)
    assert r == (None, "no_prev_point")
    # 2 時点だが間隔 10 分未満
    r = zp.select_points(
        [_jst_epoch(2026, 8, 23, 14, 58), _jst_epoch(2026, 8, 23, 15, 5)], date, post)
    assert r == (None, "no_prev_point")


def test_delta_sign_convention():
    # オッズ 10.0 → 8.0（低下 = 締まった）は Δ>0。
    assert math.log(10.0) - math.log(8.0) > 0
    assert zp.group_of(math.log(10.0) - math.log(8.0)) == "+"
    assert zp.group_of(math.log(8.0) - math.log(10.0)) == "-"
    assert zp.group_of(0.0) == "0"


def test_parse_result_odds_fixture():
    # 実ページ（202607010210）の構造を最小化したフィクスチャ。取消馬はオッズセル無し。
    html = (
        '<tr class="HorseList">'
        '<td class="Num Waku4">4</td>'
        '<td class="Num Txt_C"><div>4</div></td>'
        '<td class="Odds  Txt_C"><span>4</span></td>'
        '<td class="Odds Txt_R">\n<span  class="Odds_Ninki">8.6</span>\n</td>'
        "</tr>"
        '<tr class="HorseList">'
        '<td class="Num Txt_C"><div>7</div></td>'
        '<td class="Odds Txt_R">\n<span >11.1</span>\n</td>'
        "</tr>"
        '<tr class="HorseList">'  # 取消馬: Odds Txt_R セル無し
        '<td class="Num Txt_C"><div>9</div></td>'
        "</tr>"
    )
    assert zp.parse_result_odds(html) == {4: 8.6, 7: 11.1}


def test_strata_summary_weighted_diff():
    # 2 層 × 2 群の手計算例。bounds を直接与える（quantile ではなく規則の検証）。
    recs = [
        # bin0（ln(odds)<ln(5)）: Δ>0 の平均 200、Δ≤0 の平均 100 → 差 +100（n=4）
        {"delta": +0.1, "final_odds": 2.0, "won": 1, "ret": 200.0},
        {"delta": +0.2, "final_odds": 2.0, "won": 1, "ret": 200.0},
        {"delta": -0.1, "final_odds": 2.0, "won": 1, "ret": 100.0},
        {"delta": 0.0, "final_odds": 2.0, "won": 1, "ret": 100.0},
        # bin1: Δ>0 の平均 0、Δ≤0 の平均 50 → 差 −50（n=2）
        {"delta": +0.1, "final_odds": 9.0, "won": 0, "ret": 0.0},
        {"delta": -0.2, "final_odds": 9.0, "won": 1, "ret": 50.0},
    ]
    bounds = [math.log(5.0)]  # 2 bin に固定
    s = zp.strata_summary(recs, bounds)
    assert s["cells"][(0, "+")] == (2, 200.0)
    assert s["cells"][(0, "-")] == (1, 100.0)
    # 加重差 = (4·(+100) + 2·(−50)) / 6 = +50
    assert abs(s["weighted_diff"] - 50.0) < 1e-9
    # 低オッズ層（LOW_STRATA=(0,1)）はこの 2 bin 構成では全体と同じ
    assert abs(s["low_weighted_diff"] - 50.0) < 1e-9


def test_logistic_recovers_synthetic():
    # 既知係数 (a=-1.0, b=-1.0, δ=+0.8) の合成データを回収できること（符号と近似値）。
    # Δ の分散を実データより広め（σ=0.5）にして δ の識別力を確保する（σ=0.15 だと
    # n=6000 でも SE≈0.2 で許容±0.4 に乗らない）。
    rng = random.Random(7)
    recs = []
    for i in range(6000):
        ln_o = rng.uniform(0.0, 3.5)
        delta = rng.gauss(0.0, 0.5)
        z = -1.0 - 1.0 * ln_o + 0.8 * delta
        p = 1.0 / (1.0 + math.exp(-z))
        won = 1 if rng.random() < p else 0
        recs.append({"delta": delta, "final_odds": math.exp(ln_o), "won": won, "ret": 0.0})
    a, b, d, ok = zp.fit_logistic(recs)
    assert ok
    assert abs(a + 1.0) < 0.2 and abs(b + 1.0) < 0.2
    assert d > 0.5 and abs(d - 0.8) < 0.3


def test_parse_utc_ts_rejects_non_utc_offset():
    assert zp.parse_utc_ts("2026-08-23T06:10:00.123456+00:00") == calendar.timegm(
        (2026, 8, 23, 6, 10, 0, 0, 0, 0))
    try:
        zp.parse_utc_ts("2026-08-23T15:10:00+09:00")
    except ValueError:
        pass
    else:
        raise AssertionError("+09:00 を黙って受け入れた（9 時間ずれる）")


def test_quantile_bounds_and_assign_bin():
    xs = [float(i) for i in range(10)]  # 0..9
    b = zp.quantile_bounds(xs, 5)
    assert b == [2.0, 4.0, 6.0, 8.0]
    # 境界ちょうどは上位 bin（x < bound で判定）。
    assert zp.assign_bin(1.9, b) == 0
    assert zp.assign_bin(2.0, b) == 1
    assert zp.assign_bin(9.0, b) == 4
    bins = [zp.assign_bin(x, b) for x in xs]
    assert all(bins.count(k) == 2 for k in range(5))  # 均等に 2 個ずつ
    # タイだらけ（同一オッズ多数）でも境界が重複するだけで落ちない（空 bin は許容）。
    tied = [1.0] * 8 + [5.0, 6.0]
    bt = zp.quantile_bounds(tied, 5)
    assert all(0 <= zp.assign_bin(x, bt) <= 4 for x in tied)


def test_bootstrap_ci_covers_true_zero():
    rng = random.Random(11)
    by_race = {}
    for r in range(60):
        recs = []
        for _ in range(10):
            ln_o = rng.uniform(0.0, 3.0)
            delta = rng.gauss(0.0, 0.2)
            p = 1.0 / (1.0 + math.exp(-(-0.5 - 1.0 * ln_o)))
            recs.append({"delta": delta, "final_odds": math.exp(ln_o),
                         "won": 1 if rng.random() < p else 0, "ret": 0.0})
        by_race[f"R{r}"] = recs
    lo, hi, n_skip = zp.bootstrap_delta_ci(by_race, n_boot=80, seed=1)
    assert lo <= hi
    assert n_skip < 80
    # 真の δ=0 のデータで CI が 0 を含む（生成上の偶然で外れないよう seed 固定）。
    assert lo <= 0.0 <= hi


def test_judge_precommit_matrix():
    # 改訂案提示 = CI が 0 を跨がず正 かつ 層別加重差（全体・低オッズ層）とも正。
    assert zp.judge(0.05, 0.30, 10.0, 5.0) == "改訂案提示"
    assert zp.judge(-0.05, 0.30, 10.0, 5.0) == "現行維持"   # CI が 0 跨ぎ
    assert zp.judge(-0.30, -0.05, -10.0, -5.0) == "現行維持"  # 負方向
    assert zp.judge(0.05, 0.30, -10.0, 5.0) == "現行維持"   # 層別と食い違い
    assert zp.judge(0.05, 0.30, 10.0, -5.0) == "現行維持"   # 低オッズ層で不一致
    assert zp.judge(0.05, 0.30, float("nan"), 5.0) == "現行維持"  # NaN は現行維持側


def main() -> int:
    fails = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            try:
                fn()
                print(f"ok   {name}")
            except AssertionError as e:
                fails += 1
                print(f"FAIL {name}: {e}")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
