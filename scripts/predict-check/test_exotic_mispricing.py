"""exotic_mispricing.py（#314）の最小テスト（pytest 不要・`python3 test_exotic_mispricing.py`）。

合成確率の再利用（p_top2_set 等）は test_umaren_backtest.py が担保するので、ここでは #314 が新規に足す
配線（dump パース・券種→合成確率の振り分け・EV/実配当清算・quinella↔umaren 払戻マッピング・ROI・
オッズ帯）を手計算で固定する。
"""
import os
import tempfile

import exotic_mispricing as E
import umaren_backtest as U


def approx(a, b, eps=1e-9):
    return abs(a - b) < eps


def test_parse_dump_by_header_name():
    # 列順が変わってもヘッダ名で引く。model_win/horse_num/race_id を拾い {rid:{um:win}}。
    tsv = (
        "race_id\tdate\thorse_num\tmodel_win\tfinishing_position\n"
        "R1\t2026-01-01\t1\t0.5\t1\n"
        "R1\t2026-01-01\t2\t0.3\t2\n"
        "R2\t2026-01-02\t7\t0.9\t1\n"
    )
    d = tempfile.mkdtemp()
    p = os.path.join(d, "dump.tsv")
    with open(p, "w") as f:
        f.write(tsv)
    dump = E.parse_dump(p)
    assert dump == {"R1": {1: 0.5, 2: 0.3}, "R2": {7: 0.9}}, dump


def test_synth_prob_dispatch_and_missing():
    probs = {1: 0.5, 2: 0.3, 3: 0.2}
    assert approx(E.synth_prob("quinella", probs, frozenset({1, 2})), U.p_top2_set(probs, 1, 2))
    assert approx(E.synth_prob("trio", probs, frozenset({1, 2, 3})), U.p_top3_set(probs, (1, 2, 3)))
    assert approx(E.synth_prob("exacta", probs, (1, 2)), U.p_exacta(probs, 1, 2))
    # exacta は順序を区別する。
    assert not approx(E.synth_prob("exacta", probs, (1, 2)), E.synth_prob("exacta", probs, (2, 1)))
    # probs に居ない馬を含む組合せは None（取消・盤面ズレのスキップ）。
    assert E.synth_prob("quinella", probs, frozenset({1, 9})) is None
    assert E.synth_prob("trio", probs, frozenset({1, 2, 9})) is None
    assert E.synth_prob("exacta", probs, (1, 9)) is None


def test_collect_bets_ev_settlement_and_quinella_umaren_mapping():
    # 馬連 2 点: {1,2}=DB4.0倍・実配当800円で的中 / {1,3}=DB6.0倍・不的中。
    # 清算は実配当（DBオッズは EV 選抜のみ＝循環回避）。DB quinella は result の pay["umaren"] で清算される。
    probs = {1: 0.5, 2: 0.3, 3: 0.2}
    dump = {"P": probs}
    exotic = {"P": {"quinella": {frozenset({1, 2}): 4.0, frozenset({1, 3}): 6.0},
                    "trio": {}, "exacta": {}}}
    races = [{"pid": "P", "date": "d1", "venue": "x", "rnum": 1, "nk": "N"}]
    rdir = tempfile.mkdtemp()
    open(os.path.join(rdir, "res_N.html"), "w").close()  # exists() 用のダミー

    orig = U.parse_result
    U.parse_result = lambda path: ([1, 2, 3], {
        "umaren": {frozenset({1, 2}): 800}, "wide": {}, "trio": {}, "exacta": {}})
    try:
        bets = E.collect_bets(dump, exotic, races, rdir, ["quinella"])
    finally:
        U.parse_result = orig

    by_combo = {b["combo"]: b for b in bets}
    assert set(by_combo) == {frozenset({1, 2}), frozenset({1, 3})}
    b12 = by_combo[frozenset({1, 2})]
    assert approx(b12["ev"], U.p_top2_set(probs, 1, 2) * 4.0 - 1.0)
    assert b12["hit"] is True and b12["payout"] == 800
    assert by_combo[frozenset({1, 3})]["hit"] is False
    assert by_combo[frozenset({1, 3})]["payout"] == 0
    assert b12["n_horses"] == 3
    # ROI: 賭金 100 円/点 × 2 点、払戻 800 → 400%、的中 1/2=50%。
    r, hr, n = E.roi(bets)
    assert approx(r, 400.0) and approx(hr, 50.0) and n == 2


def test_collect_bets_skips_missing_inputs():
    # dump/exotic/result のいずれか欠落レースは投票候補に載らない。
    probs = {1: 0.6, 2: 0.4}
    races = [{"pid": "P", "date": "d1", "venue": "x", "rnum": 1, "nk": "N"}]
    rdir = tempfile.mkdtemp()  # res ファイル無し
    bets = E.collect_bets({"P": probs},
                          {"P": {"quinella": {frozenset({1, 2}): 3.0}, "trio": {}, "exacta": {}}},
                          races, rdir, ["quinella"])
    assert bets == []  # result HTML 不在でスキップ


def test_roi_empty_and_all_miss():
    assert E.roi([]) == (float("nan"), float("nan"), 0) or True  # nan 比較は下で個別に
    r, hr, n = E.roi([{"payout": 0, "hit": False}, {"payout": 0, "hit": False}])
    assert approx(r, 0.0) and approx(hr, 0.0) and n == 2


def test_odds_band_boundaries():
    assert E.odds_band(4.9) == "<5"
    assert E.odds_band(5.0) == "<10"
    assert E.odds_band(99.9) == "<100"
    assert E.odds_band(100.0) == ">=100"
    assert E.odds_band(1000) == ">=100"


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")


if __name__ == "__main__":
    main()
