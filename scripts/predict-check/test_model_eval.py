"""model_eval.py のユニットテスト（#802）。

対象: 引数 `label=path` の解釈 / ◎ の選び方（同値は馬番昇順・backtest.rs と同じ）/ レース単位の
的中・top5 包含（同着を含む）・較正の寄与（手計算の Brier / LogLoss・クランプ）/ 等幅ビン（境界値は
metrics.rs と同じ割り方）/ 集計 / 日クラスタ bootstrap（日を単位に抽出する・対応あり Δ の符号と包含）/
複数 dump のレース集合の整列 / 採否の機械判定 / 引数検証 / JSON 出力と忠実性。
乱数を使う機能（bootstrap）は seed 固定で構造的性質（1 日だけなら幅 0・同一系統の Δ は 0）を検査する。
"""

import json
import math
import sys

# CI の predict-check ジョブ（stdlib-only・`python3 <file>` 直接実行）では numpy/pytest が
# 無いため自己スキップする（test_prob_eval.py と同じ扱い）。
try:
    import numpy as np
    import pytest
except ImportError:
    if __name__ == "__main__":
        print("skip: numpy/pytest 不在（stdlib-only CI では対象外。ローカルは pytest で実行）")
        sys.exit(0)
    raise

import model_eval as me

HEADER = (
    "race_id\tdate\thorse_num\tmodel_win\tmodel_place\tmodel_show\t"
    "finishing_position\twin_odds\tpopularity\t"
    "model_win_pure\tmodel_place_pure\tmodel_show_pure"
)


def _row(rid, date, num, win, place, show, fin, odds, pure=None):
    """テスト用の行 dict。pure 省略時は blended と同じ確率。"""
    pw, pp, ps = pure if pure is not None else (win, place, show)
    return {
        "race_id": rid,
        "date": date,
        "horse_num": num,
        "model_win": win,
        "model_place": place,
        "model_show": show,
        "model_win_pure": pw,
        "model_place_pure": pp,
        "model_show_pure": ps,
        "finishing_position": fin,
        "win_odds": odds,
        "popularity": None,
    }


def _race6(rid, date, fins, odds=None):
    """6 頭立て。blended の win は馬番 1 が最大、show 順位は 1,2,3,4,5,6 の順。"""
    wins = [0.40, 0.25, 0.15, 0.10, 0.06, 0.04]
    shows = [0.90, 0.80, 0.60, 0.40, 0.20, 0.10]
    odds = odds or [2.0, 4.0, 7.0, 10.0, 20.0, 30.0]
    return [
        _row(rid, date, i + 1, wins[i], min(1.0, wins[i] * 2), shows[i], fins[i], odds[i])
        for i in range(6)
    ]


def _write_tsv(tmp_path, rows, name="dump.tsv"):
    p = tmp_path / name
    cols = HEADER.split("\t")
    with open(p, "w", encoding="utf-8") as f:
        f.write(HEADER + "\n")
        for r in rows:
            f.write("\t".join("" if r[c] is None else str(r[c]) for c in cols) + "\n")
    return str(p)


# ---------- 引数 ----------


def test_parse_dump_args_label_equals_path():
    items = me.parse_dump_args(["base=/tmp/a.tsv", "cand=/tmp/b.tsv"])
    assert items == [("base", "/tmp/a.tsv"), ("cand", "/tmp/b.tsv")]


def test_parse_dump_args_rejects_missing_label_and_duplicates():
    with pytest.raises(ValueError):
        me.parse_dump_args(["/tmp/a.tsv"])
    with pytest.raises(ValueError):
        me.parse_dump_args(["a=/x.tsv", "a=/y.tsv"])


def test_filter_window_default_is_frozen_eval_window():
    rows = [
        _row("R1", "2025-12-31", 1, 0.5, 0.7, 0.9, 1, 2.0),
        _row("R2", "2026-01-01", 1, 0.5, 0.7, 0.9, 1, 2.0),
        _row("R3", "2026-08-31", 1, 0.5, 0.7, 0.9, 1, 2.0),
        _row("R4", "2026-09-01", 1, 0.5, 0.7, 0.9, 1, 2.0),
    ]
    kept = me.filter_window(rows, me.EVAL_FROM_DEFAULT, me.EVAL_TO_DEFAULT)
    assert [r["race_id"] for r in kept] == ["R2", "R3"]
    assert me.filter_window(rows, None, None) == rows  # --all 相当


def test_main_rejects_all_with_explicit_window_and_bad_bootstrap(tmp_path):
    p = _write_tsv(tmp_path, _two_days())
    with pytest.raises(SystemExit):
        me.main([f"base={p}", "--all", "--eval-from", "2026-01-01"])
    with pytest.raises(SystemExit):
        me.main([f"base={p}", "--bootstrap", "0"])


def test_main_rejects_json_same_as_dump_or_directory(tmp_path):
    p = _write_tsv(tmp_path, _two_days())
    with pytest.raises(SystemExit):
        me.main([f"base={p}", "--json", p])
    with pytest.raises(SystemExit):
        me.main([f"base={p}", "--json", str(tmp_path)])


# ---------- ◎ と包含 ----------


def test_top_pick_breaks_ties_by_lower_horse_num():
    # 馬番 3 と 5 が同値最大 → 3 を選ぶ（backtest.rs の規則）。
    rows = [
        _row("R", "2026-01-10", 5, 0.4, 0.6, 0.8, 2, 3.0),
        _row("R", "2026-01-10", 3, 0.4, 0.6, 0.8, 1, 3.0),
        _row("R", "2026-01-10", 1, 0.2, 0.4, 0.6, 3, 5.0),
    ]
    probs = [float(r["model_win"]) for r in rows]
    assert rows[me.top_pick(rows, probs)]["horse_num"] == 3


def test_per_race_stats_hits_and_top5_inclusion():
    # 6 頭: 着順は馬番 1→3着, 2→1着, 3→2着, 4〜6 は 4,5,6。◎=馬番 1（3 着）。
    rows = _race6("R", "2026-01-10", [3, 1, 2, 4, 5, 6])
    st = me.per_race_stats(rows, me.system_probs(rows, "blended"))
    assert st.hits == {"win": 0, "place": 0, "show": 1}
    # 3 着以内馬（馬番 1,2,3）は win 順位でも show 順位でも全員 top5 内。
    assert st.placed_n == 3
    assert st.incl_win == 3
    assert st.incl_show == 3
    # 較正: win ターゲットの寄与 n は頭数。
    assert st.cal["win"][2] == 6


def test_per_race_stats_missing_finish_is_miss_and_inclusion_counts_only_finished():
    # ◎（馬番 1）が着順なし（中止）→ 全ターゲット不的中。3 着以内は馬番 6,5,4 → win 順位は 6,5,4 位で
    # top5 内は馬番 4,5 の 2 頭。
    rows = _race6("R", "2026-01-10", [None, 4, 5, 3, 2, 1])
    st = me.per_race_stats(rows, me.system_probs(rows, "blended"))
    assert st.hits == {"win": 0, "place": 0, "show": 0}
    assert st.incl_win == 2


def test_dead_heat_third_requires_all_placed_for_all3():
    # 3 着同着で 3 着以内が 4 頭（馬番 1,2,3,6）。馬番 6 は win 順位 6 位で top5 外 →
    # incl は 3 だが placed_n=4 なので「3 頭すべて」にならず、平均頭数は 3 で頭打ち。
    rows = _race6("R", "2026-01-10", [1, 2, 3, 4, 5, 3])
    st = me.per_race_stats(rows, me.system_probs(rows, "blended"))
    assert st.placed_n == 4 and st.incl_win == 3
    m = me.aggregate([st])
    assert m["top5_win_all3"] == 0.0
    assert m["top5_win_mean"] == 3.0
    # 全員が top5 内なら「3 頭すべて」。
    rows2 = _race6("R", "2026-01-10", [1, 2, 3, 3, 5, 6])  # 3 着同着が馬番 3,4（どちらも top5 内）
    st2 = me.per_race_stats(rows2, me.system_probs(rows2, "blended"))
    assert st2.placed_n == 4 and st2.incl_win == 4
    assert me.aggregate([st2])["top5_win_all3"] == 1.0


def test_rank_order_tolerates_missing_horse_num_on_ties():
    # 手編集 dump で馬番が空の行が同値首位に並んでも落ちず、馬番のある方を先に選ぶ（fidelity は dump 全体を読む）。
    rows = [
        _row("R", "2026-01-10", None, 0.4, 0.6, 0.8, 2, 3.0),
        _row("R", "2026-01-10", 3, 0.4, 0.6, 0.8, 1, 3.0),
    ]
    assert rows[me.top_pick(rows, [0.4, 0.4])]["horse_num"] == 3
    f = me.fidelity(rows)
    assert f["races"] == 1 and f["win_hit"] == 1.0


def test_all3_requires_three_placed_horses():
    # 3 着馬の着順が欠落して 3 着以内が 2 頭しか無いレースは、2 頭とも top5 でも「3 頭すべて」にしない。
    rows = _race6("R", "2026-01-10", [1, 2, None, 4, 5, 6])
    st = me.per_race_stats(rows, me.system_probs(rows, "blended"))
    assert st.placed_n == 2 and st.incl_win == 2
    m = me.aggregate([st])
    assert m["top5_win_all3"] == 0.0
    assert m["top5_win_mean"] == 2.0


def test_reliability_sparse_bin_has_no_ci():
    # 2 日 × 2 レース。6 頭中 1 頭だけが [0.4,0.5) のビン（count=4 < 10）→ 疎ビンは CI を出さず空率だけ記録する。
    rows = _two_days()
    races = me.eligible_races(rows)
    stats = {"blended": [me.per_race_stats(rr, me.system_probs(rr, "blended")) for rr in races.values()]}
    samples = me.bootstrap_samples(stats, n_boot=50, seed=1)
    bins = me.reliability_bins(stats["blended"], "win", samples["blended"]["win_bin_rate"])
    b = next(x for x in bins if x["bin"] == "[0.4,0.5)")
    assert b["count"] == 4 and b["rate_ci"] is None
    assert 0.0 <= b["rate_ci_empty_frac"] <= 1.0


def test_market_system_has_no_place_show():
    rows = _race6("R", "2026-01-10", [1, 2, 3, 4, 5, 6])
    probs = me.system_probs(rows, "market")
    assert probs["place"] is None and probs["show"] is None
    st = me.per_race_stats(rows, probs)
    assert st.incl_show is None
    assert "place" not in st.cal


# ---------- 較正（手計算） ----------


def test_calibration_matches_hand_computed_brier_and_logloss():
    # 2 頭: p=(0.75, 0.25)、勝者は馬番 1。Brier = ((0.25)² + (0.25)²)/2 = 0.0625。
    # LogLoss = (−ln 0.75 − ln 0.75)/2 = −ln 0.75。
    rows = [
        _row("R", "2026-01-10", 1, 0.75, 0.9, 0.95, 1, 1.3),
        _row("R", "2026-01-10", 2, 0.25, 0.6, 0.8, 2, 4.0),
    ]
    st = me.per_race_stats(rows, me.system_probs(rows, "blended"))
    m = me.aggregate([st])
    assert m["win_brier"] == pytest.approx(0.0625)
    assert m["win_logloss"] == pytest.approx(-math.log(0.75))


def test_logloss_clamps_zero_and_one_like_metrics_rs():
    # p=1.0 が負け、p=0.0 が勝ち → クランプ無しなら inf。ε=1e-15 で有限（−ln(1e-15) ≈ 34.5）。
    rows = [
        _row("R", "2026-01-10", 1, 1.0, 1.0, 1.0, 2, 1.1),
        _row("R", "2026-01-10", 2, 0.0, 0.0, 0.0, 1, 50.0),
    ]
    st = me.per_race_stats(rows, me.system_probs(rows, "blended"))
    m = me.aggregate([st])
    assert math.isfinite(m["win_logloss"])
    # 1−ε は倍精度で 1−1e-15 ちょうどにならない（ln(1−(1−ε)) ≈ ln(9.99e-16)）ので、桁で比べる。
    assert m["win_logloss"] == pytest.approx(-math.log(me.LOG_LOSS_EPS), rel=1e-3)


def test_bin_index_boundary_matches_metrics_rs():
    # metrics.rs は (p / 0.1) as usize。0.3/0.1 = 2.999… → ビン 2（p*10 だと 3 になり食い違う）。
    idx = me.bin_index(np.array([0.3, 0.6, 0.7, 0.0, 1.0, 0.95, 0.15]))
    assert idx.tolist() == [2, 5, 6, 0, 9, 9, 1]


def test_reliability_bins_last_bin_and_empty_bins():
    rows = [
        _row("R", "2026-01-10", 1, 1.0, 1.0, 1.0, 1, 1.1),  # p=1.0 は最終ビン
        _row("R", "2026-01-10", 2, 0.0, 0.0, 0.0, 2, 50.0),  # p=0.0 は先頭ビン
    ]
    st = me.per_race_stats(rows, me.system_probs(rows, "blended"))
    bins = me.reliability_bins([st], "win")
    assert len(bins) == me.BINS
    assert bins[9]["count"] == 1 and bins[9]["rate"] == 1.0 and bins[9]["mean_pred"] == 1.0
    assert bins[0]["count"] == 1 and bins[0]["rate"] == 0.0
    assert all(b["count"] == 0 and b["rate"] is None and b["mean_pred"] is None for b in bins[1:9])
    assert bins[9]["bin"] == "[0.9,1.0]"
    # market は place を持たないので空。blended は place もある。
    assert me.reliability_bins([me.per_race_stats(rows, me.system_probs(rows, "market"))], "place") == []
    assert len(me.reliability_bins([st], "place")) == me.BINS


# ---------- 集計と bootstrap ----------


def _two_days():
    # 1/10 は 2 レースとも ◎ 的中、1/11 は 2 レースとも ◎ 不的中（show も外す）。
    a = [_race6("A1", "2026-01-10", [1, 2, 3, 4, 5, 6]), _race6("A2", "2026-01-10", [1, 3, 2, 4, 5, 6])]
    b = [_race6("B1", "2026-01-11", [6, 5, 4, 3, 2, 1]), _race6("B2", "2026-01-11", [5, 6, 4, 3, 2, 1])]
    return [r for race in a + b for r in race]


def test_aggregate_rates_match_hand_calculation():
    rows = _two_days()
    races = me.eligible_races(rows)
    stats = [me.per_race_stats(rr, me.system_probs(rr, "blended")) for rr in races.values()]
    m = me.aggregate(stats)
    assert m["races"] == 4
    assert m["win_hit"] == pytest.approx(0.5)
    assert m["show_hit"] == pytest.approx(0.5)
    # 1/10 の 2 レースは 3 着以内が馬番 1-3（全員 top5）、1/11 は 4,5,6 → win 順位 4,5,6 位で 2 頭。
    assert m["top5_win_mean"] == pytest.approx((3 + 3 + 2 + 2) / 4)
    assert m["top5_win_all3"] == pytest.approx(0.5)


def test_exclusion_counts_and_horse_num_missing():
    rows = _two_days()
    # B2 の 1 頭の馬番を欠落させる → 勝者一意・オッズ完全だが馬番欠落で母集合から外れる。
    for r in rows:
        if r["race_id"] == "B2" and r["horse_num"] == 6:
            r["horse_num"] = None
    # A2 の 1 頭のオッズを欠落させる → オッズ不完全。
    for r in rows:
        if r["race_id"] == "A2" and r["horse_num"] == 2:
            r["win_odds"] = None
    ex = me.exclusion_counts(rows)
    assert ex == {"dead_heat": 0, "no_winner": 0, "odds_incomplete": 1, "horse_num_missing": 1}
    assert list(me.eligible_races(rows)) == ["A1", "B1"]


def test_day_bootstrap_single_day_has_zero_width():
    rows = [r for race in (_race6("A1", "2026-01-10", [1, 2, 3, 4, 5, 6]),
                           _race6("A2", "2026-01-10", [2, 1, 3, 4, 5, 6])) for r in race]
    races = me.eligible_races(rows)
    stats = {"blended": [me.per_race_stats(rr, me.system_probs(rr, "blended")) for rr in races.values()]}
    ci = me.day_bootstrap(stats, n_boot=50, seed=1)
    for lo, hi in ci["blended"].values():
        assert lo == pytest.approx(hi)


def test_day_bootstrap_samples_days_not_races():
    # 2 日で日ごとに成績が割れている。日単位抽出なら win_hit の再標本値は {0, 0.5, 1} しか取らない。
    rows = _two_days()
    races = me.eligible_races(rows)
    stats = {"blended": [me.per_race_stats(rr, me.system_probs(rr, "blended")) for rr in races.values()]}
    samples = me.bootstrap_samples(stats, n_boot=200, seed=3)
    vals = set(round(v, 6) for v in samples["blended"]["win_hit"])
    assert vals <= {0.0, 0.5, 1.0}
    assert len(vals) >= 2  # 抽出が効いている
    # 帯ごとの実現率も再標本化されている（(n_boot, BINS)）。
    assert samples["blended"]["win_bin_rate"].shape == (200, me.BINS)


def test_paired_delta_of_identical_systems_is_exactly_zero():
    rows = _two_days()
    races = me.eligible_races(rows)
    st = [me.per_race_stats(rr, me.system_probs(rr, "blended")) for rr in races.values()]
    stats = {"a": st, "b": list(st)}
    d = me.delta_ci(stats, "b", "a", n_boot=100, seed=5)
    for point, lo, hi in d.values():
        assert point == 0.0 and lo == 0.0 and hi == 0.0


def test_paired_delta_sign_and_ci_contain_point():
    # 候補 = 基準と同じ 4 レースだが、1/11 の 2 レースだけ ◎ が当たる（馬番 6 の win を最大に差し替え。
    # B1・B2 とも馬番 6 が 1 着）。
    base_rows = _two_days()
    cand_rows = _two_days()
    for r in cand_rows:
        if r["race_id"] in ("B1", "B2"):
            r["model_win"] = 0.9 if r["horse_num"] == 6 else 0.02
    base = me.eligible_races(base_rows)
    cand = me.eligible_races(cand_rows)
    stats = {
        "base": [me.per_race_stats(rr, me.system_probs(rr, "blended")) for rr in base.values()],
        "cand": [me.per_race_stats(rr, me.system_probs(rr, "blended")) for rr in cand.values()],
    }
    d = me.delta_ci(stats, "cand", "base", n_boot=200, seed=7)
    point, lo, hi = d["win_hit"]
    assert point == pytest.approx(0.5)  # 基準 2/4 → 候補 4/4
    assert lo <= point <= hi
    # 1 日の全レースが同時に入れ替わるので、Δ の再標本値は {0（1/10 だけ）, 0.5（両日）, 1.0（1/11 だけ）} に収まる。
    samples = me.bootstrap_samples(stats, n_boot=200, seed=7)
    vals = set(round(v, 6) for v in (samples["cand"]["win_hit"] - samples["base"]["win_hit"]))
    assert vals <= {0.0, 0.5, 1.0}
    # 2 日しか無いので CI は 0 を跨ぎ、決め手が立たない → 判定不能（悪化側で有意な指標も無い）。
    assert me.judge(d) == "判定不能"


# ---------- 複数 dump の整列と採否判定 ----------


def test_align_race_sets_intersection_and_dropped_counts():
    base = me.eligible_races(_two_days())
    cand_rows = [r for r in _two_days() if r["race_id"] != "B2"]
    cand = me.eligible_races(cand_rows)
    common, dropped = me.align_race_sets({"base": base, "cand": cand})
    assert set(common) == {"A1", "A2", "B1"}
    assert dropped == {"base": 1, "cand": 0}


def test_judge_rules():
    # 主指標: show_hit↑ win_hit↑ top5_win_all3↑ win_logloss↓
    good = {"show_hit": (0.02, 0.01, 0.03), "win_hit": (0.0, -0.01, 0.01),
            "top5_win_all3": (0.0, -0.02, 0.02), "win_logloss": (-0.001, -0.002, 0.0005)}
    assert me.judge(good) == "採用候補"
    worse = dict(good, win_logloss=(0.01, 0.005, 0.02))  # LogLoss が有意に悪化
    assert me.judge(worse) == "悪化"
    unclear = dict(good, show_hit=(0.02, -0.01, 0.05))
    assert me.judge(unclear) == "判定不能"


# ---------- 忠実性と end-to-end ----------


def test_fidelity_matches_hand_calculation():
    rows = _two_days()
    f = me.fidelity(rows)
    assert f["races"] == 4
    assert f["win_hit"] == pytest.approx(0.5)
    assert f["show_hit"] == pytest.approx(0.5)
    # 全 24 頭の blended win Brier / LogLoss を独立に計算して突合。
    p = np.array([r["model_win"] for r in rows])
    y = np.array([1.0 if r["finishing_position"] == 1 else 0.0 for r in rows])
    assert f["win_brier"] == pytest.approx(float(np.mean((p - y) ** 2)))
    pc = np.clip(p, 1e-15, 1 - 1e-15)
    assert f["win_logloss"] == pytest.approx(float(np.mean(-(y * np.log(pc) + (1 - y) * np.log(1 - pc)))))


def test_main_writes_json_with_fidelity_market_reference_and_judgment(tmp_path, capsys):
    base = _write_tsv(tmp_path, _two_days(), "base.tsv")
    cand = _write_tsv(tmp_path, _two_days(), "cand.tsv")
    out = str(tmp_path / "out.json")
    me.main([f"base={base}", f"cand={cand}", "--bootstrap", "30", "--json", out])
    text = capsys.readouterr().out
    assert "忠実性" in text and "採否" in text and "市場との差" in text
    with open(out, encoding="utf-8") as f:
        j = json.load(f)
    assert j["population"]["races"] == 4
    assert set(j["systems"]) >= {"market", "pure", "blended", "cand:pure", "cand:blended"}
    assert j["fidelity"]["base"]["win_hit"] == pytest.approx(0.5)  # 忠実性は dump ごと（blended の値）
    # 市場との差は market 以外の全系統（候補の pure も含む）に出て、採否の判定は付かない。
    assert set(j["market_reference"]) == {"pure", "blended", "cand:pure", "cand:blended"}
    assert "verdict" not in j["market_reference"]["blended"]
    assert j["judgments"]["cand:blended"]["verdict"] == "判定不能"  # 同一 dump なので Δ=0
    assert j["dumps"][0]["sha256_12"] == me.file_sha256(base)
    assert "rate_ci" in next(b for b in j["reliability"]["blended"]["win"] if b["count"])


if __name__ == "__main__":
    sys.exit(pytest.main([__file__, "-q"]))
