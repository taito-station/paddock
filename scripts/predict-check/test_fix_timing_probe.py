"""fix_timing_probe.py の最小テスト（pytest 不要・`python3 test_fix_timing_probe.py`）.

#724 の「朝 vs T-40」計測で、集計を静かに汚しうる箇所を assert で固定する:
非出走（取消・除外）の判定と返還つき精算／計測対象から外す理由／対の差のブートストラップの再現性／
事前に固定した判定規則（CI が 0 を跨がない かつ 差が 0 でない日の 7 割以上で向きが一致）／MDE の式。
DB にも netkeiba にも触らない。
"""
import io
import os
import tempfile
from contextlib import redirect_stdout

import fix_timing_probe as F


def approx(a, b, eps=1e-9):
    return abs(a - b) < eps


def row(rank_cell, num):
    return (
        '<tr class="HorseList"><td class="Result_Num"><div class="Rank">'
        + rank_cell
        + '</div></td><td class="Num Waku1"><div>1</div></td>'
        + f'<td class="Num Txt_C"><div>{num}</div></td></tr>'
    )


def test_non_runners_are_scratch_and_exclusion_only():
    # 取消・除外は返還の対象。競走中止は走っているので返還しない。
    html = row("1", 3) + row("2", 5) + row("取消", 7) + row("除外", 8) + row("中止", 9)
    assert F.non_runners(html) == {7, 8}


def test_finish_ranks_ignores_trailing_rows_from_other_tables():
    # 古いテンプレートでは結果表の後ろに名前の無い行が残り、1 着馬の馬番で読めてしまう
    rows = [
        {"rank": 1, "horse_num": 9, "name": "シーズザスローン"},
        {"rank": 2, "horse_num": 12, "name": "タガノエルー"},
        {"rank": None, "horse_num": 9, "name": ""},
        {"rank": None, "horse_num": 9, "name": ""},
    ]
    assert F.finish_ranks(rows) == {9: 1, 12: 2}


def leg(bt, combo, amount):
    return {"bet_type": bt, "combo": combo, "amount": amount}


def test_settle_refunds_legs_with_non_runners():
    payouts = {"quinella": {"1-2": 800}, "wide": {"1-3": 300}}
    legs = [
        leg("quinella", [1, 2], 500),  # 的中 500/100×800 = 4000
        leg("wide", [1, 3], 500),  # 的中 500/100×300 = 1500
        leg("wide", [1, 7], 500),  # 7 番が取消 → 返還（賭金にも払戻にも入れない）
        leg("trio", [1, 2, 4], 1000),  # 不的中
        leg("trio", [1, 2, 6], 0),  # ¥0 の脚は賭けていない
    ]
    stake, ret, refunded = F.settle_with_refund(legs, payouts, {7})
    assert approx(stake, 2000)
    assert approx(ret, 5500)
    assert refunded == 1


def rec(**kw):
    base = {
        "race_id": "2026-2-niigata-3-10R",
        "skip": None,
        "captured_at": "2026-08-15T05:07:17Z",
        "t40_at": "2026-08-15T05:08:30.5+00:00",
        "morning_at": "2026-08-15T01:00:00.123+00:00",
        "t40": {"has_ev": True, "legs": []},
        "morning_fixed": {"has_ev": True, "legs": []},
    }
    base.update(kw)
    return base


def test_exclusion_reasons():
    assert F.exclusion_reason(rec()) is None
    assert F.exclusion_reason(rec(skip="朝のスナップショットが無い")) == "skip"
    # live は EV が出ないレースをスキップするので、計測からも外す
    assert F.exclusion_reason(rec(t40={"has_ev": False, "legs": []})) == "no_ev"
    # 朝と T-40 の差が 60 分未満（朝の選定と初回スイープがほぼ同時）は外す
    assert F.exclusion_reason(rec(morning_at="2026-08-15T04:30:00+00:00")) == "gap"
    assert F.exclusion_reason(rec(morning_at="2026-08-15T04:07:17+00:00")) is None  # ちょうど 60 分
    # 初回スイープの保存が無く、次のスイープ（5 分後）の全券種を拾った
    assert F.exclusion_reason(rec(t40_at="2026-08-15T05:12:17.5+00:00")) == "t40_late"
    assert F.exclusion_reason(rec(t40_at="2026-08-15T05:12:17+00:00")) is None  # ちょうど 300 秒


def test_cached_result_is_read_without_deleting_truncated_pages():
    with tempfile.TemporaryDirectory() as d:
        ok, cut = os.path.join(d, "202605040111.html"), os.path.join(d, "202605040112.html")
        with open(ok, "wb") as f:
            f.write(b"<html><body>ok</body></html>")
        with open(cut, "wb") as f:
            f.write(b"<html><body>cut")
        html, why = F.read_cached_result(d, "202605040111")
        assert "ok" in html and why is None
        assert F.read_cached_result(d, "202605040112") == (None, "truncated_cache")
        assert os.path.exists(cut)  # 計測はキャッシュを消さない
        assert F.read_cached_result(d, "202605040113") == (None, "no_payout")
        assert F.read_cached_result(d, "../x") == (None, "no_payout")


def race(day, sm, rm, st, rt):
    return {"day": day, "m_stake": sm, "m_ret": rm, "t_stake": st, "t_ret": rt}


def test_pooled_diff_and_bootstrap_are_reproducible():
    rs = [race("d1", 5000, 9000, 5000, 3000), race("d1", 5000, 0, 5000, 4000)]
    # 朝 ROI 0.9 − T-40 ROI 0.7 = +0.2
    assert approx(F.pooled_diff(rs), 0.2)
    a = F.bootstrap_diff_ci(rs, iters=200, seed=571)
    b = F.bootstrap_diff_ci(rs, iters=200, seed=571)
    assert a == b
    lo, hi = a
    assert lo <= 0.2 <= hi


def test_report_survives_races_without_stake():
    # 朝の側に賭金が無い（差が出せない）標本でも落ちずに知らせる
    rs = [race("d1", 0, 0, 5000, 3000), race("d1", 0, 0, 5000, 4000), race("d2", 0, 0, 5000, 0)]
    out = io.StringIO()
    with redirect_stdout(out):
        F.report(rs, "t")
    assert "足りない" in out.getvalue()
    # 3 レースだと「上位 3 レースを除く」が空になる
    extra = {"refunded": 0, "axis_diff": False, "partner_diff": False, "band_diff": False,
             "m_axis_rank": 1, "t_axis_rank": 2, "morning_hour": 9, "gap_min": 90}
    rs = [dict(race("d1", 5000, 9000, 5000, 3000), **extra), dict(race("d1", 5000, 0, 5000, 4000), **extra),
          dict(race("d2", 5000, 100, 5000, 0), **extra)]
    out = io.StringIO()
    with redirect_stdout(out):
        F.report(rs, "t")
    assert "上位 3 レースを除く差: —" in out.getvalue()


def recorded_race(recorded, t40, win):
    return {"recorded": recorded, "t40": t40, "t40_blended_win": win,
            "captured_at": "2026-08-15T05:07:17Z", "t40_at": "2026-08-15T05:08:17+00:00"}


def test_mismatch_does_not_double_count_an_axis_swap_as_partners():
    r = recorded_race(
        {"axis": 4, "partners": [1, 3, 6, 9, 11], "konsen_band": []},
        {"axis": 9, "partners": [4, 3, 11, 6, 1], "legs": []},
        [[9, 0.255], [4, 0.2499]],
    )
    assert F.mismatch(r) == [("軸", [4], [9])]


def test_classify_explains_near_ties_by_input_drift_only():
    win = [[5, 0.26], [6, 0.0297], [1, 0.0262]]
    r = recorded_race({}, {}, win)
    # 入れ替わった馬の勝率の差 0.35pt がその日のずれ 1.3pt 以内 → 入力の差
    assert F.classify(r, "相手", [1], [6], 1.3).startswith("(b)/(c)")
    assert F.classify(r, "相手", [1], [6], 0.2).startswith("未分類")
    # 2 頭どうしの入れ替わりは最も遠い組で測る（⑥ 3.0% と ② 0.5% → 2.5pt）
    r2 = recorded_race({}, {}, [[6, 0.030], [7, 0.029], [1, 0.028], [2, 0.005]])
    assert F.classify(r2, "相手", [1, 2], [6, 7], 1.0).startswith("未分類（勝率の差 2.50pt")
    late = dict(r, t40_at="2026-08-15T05:13:17+00:00")
    assert F.classify(late, "相手", [1], [6], 1.3) == "(a) 時点"


def test_partner_diff_excludes_axis_swap():
    a = {"axis": 4, "partners": [1, 3, 6, 9, 11]}
    b = {"axis": 9, "partners": [4, 3, 11, 6, 1]}
    assert not F._partners_differ(a, b)
    assert F._partners_differ(a, {"axis": 4, "partners": [1, 3, 6, 9, 12]})


def test_main_rejects_axis_prob_without_value():
    assert F.main(["--check-recorded", "--axis-prob"]) == 2


def test_judge_needs_both_ci_and_day_direction():
    # CI が 0 を跨がず、差が 0 でない日のうち 7 割以上が同じ向き → 前倒しを提案
    assert F.judge(ci=(0.05, 0.30), day_diffs=[0.1] * 7 + [-0.1] * 3) == "前倒しを提案"
    # 向きの一致が 7 割未満
    assert F.judge(ci=(0.05, 0.30), day_diffs=[0.1] * 6 + [-0.1] * 4) == "T-40 を維持"
    # CI が 0 を跨ぐ
    assert F.judge(ci=(-0.05, 0.30), day_diffs=[0.1] * 10) == "T-40 を維持"
    # 差 0 の日は数えない（7/7 の一致）
    assert F.judge(ci=(0.01, 0.2), day_diffs=[0.1] * 7 + [0.0] * 3) == "前倒しを提案"
    # 朝のほうが悪い向きで CI が負に寄っても、前倒しは提案しない
    assert F.judge(ci=(-0.3, -0.05), day_diffs=[-0.1] * 10) == "T-40 を維持"


def test_mde_matches_formula():
    # (1.96 + 0.8416) × √2 × sd / √n
    assert approx(F.mde(sd=1.0, n=100), (1.959964 + 0.841621) * 2 ** 0.5 / 10, eps=1e-4)


if __name__ == "__main__":
    for name, fn in list(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print(f"ok  {name}")
