"""gate_calibration.py の最小テスト（pytest 不要・`python3 test_gate_calibration.py`）.

#571 のゲート較正測定で、集計を静かに汚しうる箇所の不変量を assert で固定する:
精算（ワイド 3 組・不的中・¥0 脚）／JST 正規化と発走前スイープの選択／市場整合ROI が
1−控除率 に一致すること／バケット・閾値表の境界。DB にも netkeiba にも触らない。
"""
import gate_calibration as G


def approx(a, b, eps=1e-9):
    return abs(a - b) < eps


def test_combo_key_is_numeric_ascending():
    # 文字列ソートだと "10" < "5" になり 3 連複のキーが崩れる。数値昇順であること。
    assert G.combo_key([5, 10, 1]) == "1-5-10"
    assert G.combo_key([10, 5]) == "5-10"
    # 入力順に依存しない（伝票の combo は軸が先頭のこともある）
    assert G.combo_key([13, 7]) == G.combo_key([7, 13])
    # 文字列で来ても数値として扱う
    assert G.combo_key(["10", "2"]) == "2-10"


def test_settle_counts_all_three_wide_hits():
    # ワイドは 3 着以内の 2 頭組が 3 通り当たる。伝票が複数本当てたら全部拾う。
    legs = [
        {"bet_type": "wide", "combo": [1, 2], "amount": 300},
        {"bet_type": "wide", "combo": [1, 3], "amount": 300},
        {"bet_type": "wide", "combo": [2, 3], "amount": 300},
        {"bet_type": "wide", "combo": [1, 9], "amount": 300},  # 不的中
    ]
    payouts = {"wide": {"1-2": 400, "1-3": 500, "2-3": 900}}
    stake, ret, hits, by_type = G.settle(legs, payouts)
    assert stake == 1200
    assert approx(ret, 3 * 400 + 3 * 500 + 3 * 900)  # amount/100 × payout
    assert hits == 3
    assert by_type["wide"]["legs"] == 4 and by_type["wide"]["hits"] == 3


def test_settle_excludes_zero_amount_legs_from_denominator():
    # 券種予算を賄えず ¥0 になった脚は「買っていない」ので分母にも分子にも入らない。
    legs = [
        {"bet_type": "quinella", "combo": [1, 2], "amount": 0},
        {"bet_type": "quinella", "combo": [1, 3], "amount": 500},
    ]
    stake, ret, hits, by_type = G.settle(legs, {"quinella": {"1-2": 9999}})
    assert stake == 500
    assert ret == 0.0 and hits == 0
    assert by_type["quinella"]["legs"] == 1


def test_settle_handles_missing_bet_type_without_raising():
    # 払戻に該当券種のブロックが無い（＝その券種は不的中/未掲載）でも例外にせず 0 円計上。
    legs = [{"bet_type": "trio", "combo": [1, 2, 3], "amount": 200}]
    stake, ret, hits, _ = G.settle(legs, {"win": {"1": 300}})
    assert stake == 200 and ret == 0.0 and hits == 0


def test_settle_matches_hand_calculation():
    # 手計算との一致: 300 円で 1,240 円配当 → 300/100 × 1240 = 3,720 円。
    legs = [{"bet_type": "quinella", "combo": [4, 11], "amount": 300},
            {"bet_type": "trio", "combo": [4, 11, 2], "amount": 200}]
    stake, ret, _, _ = G.settle(legs, {"quinella": {"4-11": 1240}})
    assert stake == 500
    assert approx(ret, 3720.0)
    assert approx(ret / stake * 100.0, 744.0)


def test_to_jst_normalizes_utc_suffix():
    # captured_at は UTC の `...Z`。JST は +9h。
    dt = G.to_jst("2026-08-09T09:27:37Z")
    assert (dt.hour, dt.minute) == (18, 27)
    # オフセット表記（psql の timestamptz 出力）でも同じ時刻になる
    assert G.to_jst("2026-08-09T09:27:37+00:00") == dt


def test_post_dt_and_pre_post_boundary():
    # 発走ちょうどのスイープは「発走前」に含め、1 秒でも過ぎたら除く。
    rows = [
        {"date": "2026-08-09", "post_time": "18:30", "captured_at": "2026-08-09T09:15:00Z", "roi": 50.0},
        {"date": "2026-08-09", "post_time": "18:30", "captured_at": "2026-08-09T09:30:00Z", "roi": 10.0},
        {"date": "2026-08-09", "post_time": "18:30", "captured_at": "2026-08-09T09:30:01Z", "roi": 99.0},
    ]
    final, ever = G.pick_race_rows(rows)
    assert final["captured_at"] == "2026-08-09T09:30:00Z"  # 発走ちょうど = 発走前の最終
    assert ever["roi"] == 50.0  # 発走後の 99.0 は ever にも入れない


def test_pick_race_rows_rejects_unknown_post_time():
    # 発走時刻が無いレースは判断材料にならないので落とす（黙って全スイープを使わない）。
    rows = [{"date": "2026-08-09", "post_time": "", "captured_at": "2026-08-09T09:15:00Z", "roi": 50.0}]
    assert G.pick_race_rows(rows) == (None, None)
    # 発走前スイープが 1 本も無い場合も同様
    rows = [{"date": "2026-08-09", "post_time": "10:00", "captured_at": "2026-08-09T09:15:00Z", "roi": 50.0}]
    assert G.pick_race_rows(rows) == (None, None)


def test_market_fair_roi_equals_one_minus_takeout():
    # 控除率 t の市場を人工的に作る: Σ(1/O) = W/(1-t) になるよう均等オッズを置く。
    # このとき market_fair_roi はどんな買い方でも (1-t) に一致する（正規化の不変量）。
    for bet_type, w, t in (("quinella", 1, 0.225), ("trio", 1, 0.25), ("wide", 3, 0.225)):
        n_combos = 20
        odds = n_combos * (1 - t) / w  # 均等オッズ: Σ(1/O) = n/odds = W/(1-t)
        table = {f"{i}-{i+100}": odds for i in range(n_combos)}
        legs = [{"bet_type": bet_type, "combo": [0, 100], "amount": 300},
                {"bet_type": bet_type, "combo": [3, 103], "amount": 700}]
        got = G.market_fair_roi(legs, {bet_type: table})
        assert approx(got, 1 - t, 1e-9), (bet_type, got, 1 - t)


def test_market_fair_roi_is_allocation_invariant():
    # 配分を変えても（均等オッズ市場では）市場整合ROIは動かない＝控除率だけを測っている。
    table = {f"{i}-{i+100}": 15.5 for i in range(20)}
    mk = lambda a1, a2: [{"bet_type": "quinella", "combo": [0, 100], "amount": a1},
                         {"bet_type": "quinella", "combo": [1, 101], "amount": a2}]
    a = G.market_fair_roi(mk(100, 900), {"quinella": table})
    b = G.market_fair_roi(mk(500, 500), {"quinella": table})
    assert approx(a, b)


def test_market_fair_roi_none_when_odds_missing():
    # オッズ表が無い＝評価不能。0 や 1 を返して集計に混ぜない。
    assert G.market_fair_roi([{"bet_type": "wide", "combo": [1, 2], "amount": 300}], {}) is None


def test_bucketize_edges_are_lower_inclusive():
    mk = lambda v: {"judged_roi": v, "stake": 100.0, "ret": 0.0, "realized_roi": 0.0}
    races = [mk(19.9), mk(20.0), mk(39.9), mk(80.0), mk(120.0)]
    labels, buckets = G.bucketize(races, [20, 40, 60, 80])
    assert labels == ["<20%", "20–40%", "40–60%", "60–80%", "≥80%"]
    assert [len(b) for b in buckets] == [1, 2, 0, 0, 2]


def test_pooled_roi_and_threshold_table():
    races = [
        {"judged_roi": 10.0, "stake": 1000.0, "ret": 0.0},
        {"judged_roi": 50.0, "stake": 1000.0, "ret": 2000.0},
        {"judged_roi": 90.0, "stake": 1000.0, "ret": 1000.0},
    ]
    assert approx(G.pooled_roi(races), 100.0)  # 3000/3000
    table = dict((th, (n, roi)) for th, n, roi in G.threshold_table(races, [0, 50, 90, 100]))
    assert table[0][0] == 3
    assert table[50][0] == 2 and approx(table[50][1], 150.0)  # 3000/2000
    assert table[90][0] == 1 and approx(table[90][1], 100.0)
    assert table[100] == (0, None)  # 通過ゼロは None（0% と区別する）


def test_bootstrap_ci_is_deterministic_and_brackets_point_estimate():
    races = [{"judged_roi": 0.0, "stake": 1000.0, "ret": r} for r in
             (0.0, 0.0, 0.0, 500.0, 900.0, 1200.0, 5000.0, 0.0, 300.0, 800.0)]
    lo1, hi1 = G.bootstrap_ci(races)
    lo2, hi2 = G.bootstrap_ci(races)
    assert (lo1, hi1) == (lo2, hi2)  # seed 固定で再現可能
    point = G.pooled_roi(races)
    assert lo1 <= point <= hi1


def test_nearest_odds_picks_latest_at_or_before_capture():
    odds = {
        "2026-08-09T08:00:00+00:00": {"quinella": {"1-2": 10.0}},
        "2026-08-09T09:00:00+00:00": {"quinella": {"1-2": 20.0}},
        "2026-08-09T10:00:00+00:00": {"quinella": {"1-2": 30.0}},
    }
    assert G.nearest_odds(odds, "2026-08-09T09:30:00Z")["quinella"]["1-2"] == 20.0
    assert G.nearest_odds(odds, "2026-08-09T09:00:00Z")["quinella"]["1-2"] == 20.0
    # captured_at より前のオッズが 1 本も無ければ最古で代替する（空表を返して黙って落とさない）
    assert G.nearest_odds(odds, "2026-08-09T07:00:00Z")["quinella"]["1-2"] == 10.0
    assert G.nearest_odds({}, "2026-08-09T09:30:00Z") == {}


def test_load_live_ev_skips_broken_rows():
    good = "\t".join(["r1", "2026-08-09", "chukyo", "12", "18:30", "2026-08-09T09:00:00Z",
                      "16.25", "13", "f", "f", '{"legs": [], "race_budget": 5000}'])
    rows = G.load_live_ev(good + "\n" + "too\tfew\tcols\n" + "\n")
    assert list(rows) == ["r1"] and len(rows["r1"]) == 1
    assert rows["r1"][0]["roi"] == 16.25 and rows["r1"][0]["race_no"] == 12
    assert rows["r1"][0]["axis"] == 13
    assert rows["r1"][0]["konsen"] is False and rows["r1"][0]["slip"]["race_budget"] == 5000


def test_axis_stability_counts_only_pre_post_flips():
    mk = lambda cap, axis, roi: {"date": "2026-08-01", "venue": "sapporo", "race_no": 7,
                                 "post_time": "13:20", "captured_at": cap, "axis": axis, "roi": roi}
    by_race = {
        # 発走前に軸が 3→6 へ入れ替わる
        "flip": [mk("2026-08-01T02:52:20Z", 3, 68.0), mk("2026-08-01T04:15:59Z", 6, 31.8)],
        # 発走前は軸不変、発走後だけ別軸 → フリップに数えない
        "post-only": [mk("2026-08-01T02:52:20Z", 3, 68.0), mk("2026-08-01T03:00:00Z", 3, 60.0),
                      mk("2026-08-01T05:00:00Z", 9, 10.0)],
        # 発走前スイープが 1 本だけ → 母数に入れない
        "single": [mk("2026-08-01T02:52:20Z", 3, 68.0)],
    }
    multi, flipped = G.axis_stability(by_race)
    assert multi == 2
    assert len(flipped) == 1
    assert flipped[0][2] == [3, 6]  # 出現順の軸遷移


def test_axis_stability_keeps_round_trip_visible():
    # 3→9→3 と往復した場合、最初と最後だけ見ると同じ軸に見える。遷移列で可視化されること。
    mk = lambda cap, axis: {"date": "2026-08-01", "venue": "sapporo", "race_no": 10,
                            "post_time": "13:20", "captured_at": cap, "axis": axis, "roi": 20.0}
    by_race = {"rt": [mk("2026-08-01T02:00:00Z", 3), mk("2026-08-01T03:00:00Z", 9),
                      mk("2026-08-01T04:00:00Z", 3)]}
    multi, flipped = G.axis_stability(by_race)
    assert multi == 1 and len(flipped) == 1
    assert flipped[0][2] == [3, 9, 3]


def test_load_odds_uses_wide_band_midpoint():
    tsv = ("r1\twide\t1-2\t40.8\t43.7\t2026-08-09T09:00:00+00:00\n"
           "r1\tquinella\t1-2\t200.9\t\t2026-08-09T09:00:00+00:00\n")
    odds = G.load_odds(tsv)
    tbl = odds["r1"]["2026-08-09T09:00:00+00:00"]
    assert approx(tbl["wide"]["1-2"], (40.8 + 43.7) / 2)
    assert approx(tbl["quinella"]["1-2"], 200.9)


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")


if __name__ == "__main__":
    main()
