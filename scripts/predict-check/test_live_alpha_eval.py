"""live_alpha_eval.py のユニットテスト（#218）。

対象: スナップショットの時点選択（JST 開催日・発走前・全馬そろう完全スナップショットだけ。朝 / T-40 / 直前）/
dump 行のオッズ差し替えと α=0.2・γ=1.25 の再構成 / 事前コミットした判定規則（朝で「採用候補」なら維持、
それ以外は α=0）/ 3 時点で同じレース集合にそろえること。DB には触らない（スナップショットは手作り）。
"""

import sys

try:
    import numpy as np
    import pytest
except ImportError:
    if __name__ == "__main__":
        print("skip: numpy/pytest 不在（stdlib-only CI では対象外。ローカルは pytest で実行）")
        sys.exit(0)
    raise

import live_alpha_eval as la
import mag_sweep as ms

JST = 9 * 3600


def _epoch_jst(date, hhmm):
    """JST の 'YYYY-MM-DD' と 'HH:MM' を UTC epoch 秒に。"""
    import calendar
    import time

    y, mo, d = map(int, date.split("-"))
    h, mi = map(int, hhmm.split(":"))
    return calendar.timegm(time.struct_time((y, mo, d, h, mi, 0, 0, 0, 0))) - JST


def _entry(date, post, snaps):
    """fetch_snapshots_with_cards の 1 レース分（snaps: {'HH:MM' or (date,'HH:MM'): {馬番: odds}}）。"""
    out = {}
    for k, v in snaps.items():
        d, t = (k if isinstance(k, tuple) else (date, k))
        out[_epoch_jst(d, t)] = v
    return {"date": date, "post_min": la.zsp.hhmm_to_min(post), "snaps": out}


FULL = {1: 2.0, 2: 4.0, 3: 8.0}


# ---------- 時点選択 ----------


def test_select_points_morning_t40_last():
    e = _entry("2026-07-04", "15:00", {
        "09:00": FULL,
        "10:00": {1: 2.2, 2: 3.8, 3: 7.5},
        "14:30": {1: 2.4, 2: 3.6, 3: 7.0},  # 発走 30 分前 = T-40 以降で最初
        "14:58": {1: 2.5, 2: 3.5, 3: 6.9},  # 直前
    })
    pts = la.select_points(e, {1, 2, 3})
    assert pts["morning"] == FULL
    assert pts["t40"] == {1: 2.4, 2: 3.6, 3: 7.0}
    assert pts["last"] == {1: 2.5, 2: 3.5, 3: 6.9}


def test_select_points_t40_boundary_is_inclusive():
    # ちょうど 40 分前の取得は T-40 に入る（事前コミットの定義。本番の FirstCompleteSince の近似）。
    e = _entry("2026-07-04", "15:00", {"09:00": FULL, "14:20": {1: 2.1, 2: 4.1, 3: 8.1}, "14:50": FULL})
    assert la.select_points(e, {1, 2, 3})["t40"] == {1: 2.1, 2: 4.1, 3: 8.1}


def test_select_points_skips_incomplete_post_race_and_other_day():
    e = _entry("2026-07-04", "15:00", {
        ("2026-07-03", "20:00"): FULL,   # 前日（JST 開催日と違う）→ 使わない
        "09:00": {1: 2.0, 2: 4.0},       # 馬 3 が欠ける → 不完全
        "09:15": {1: 2.1, 2: 4.1, 3: 8.1},  # 朝の最初の完全スナップショット
        "14:50": {1: 2.2, 2: 4.2, 3: 8.2},
        "15:05": {1: 9.9, 2: 9.9, 3: 9.9},  # 発走後 → 使わない
        ("2026-07-05", "10:00"): FULL,   # 翌日の再取得 → 使わない
    })
    pts = la.select_points(e, {1, 2, 3})
    assert pts["morning"] == {1: 2.1, 2: 4.1, 3: 8.1}
    assert pts["last"] == {1: 2.2, 2: 4.2, 3: 8.2}


def test_select_points_none_when_no_t40_or_no_post_time():
    e = _entry("2026-07-04", "15:00", {"09:00": FULL, "10:00": FULL})  # T-40 以降の取得が無い
    pts = la.select_points(e, {1, 2, 3})
    assert pts["morning"] == FULL and pts["t40"] is None
    e2 = _entry("2026-07-04", None, {"09:00": FULL})
    assert la.select_points(e2, {1, 2, 3}) is None


# ---------- 差し替えと再構成 ----------


def _rows(rid="R", date="2026-07-04"):
    pure = [0.5, 0.3, 0.2]
    rows = []
    for i, num in enumerate([1, 2, 3]):
        rows.append({
            "race_id": rid, "date": date, "horse_num": num,
            "model_win": None, "model_place": 0.6, "model_show": 0.8,
            "model_win_pure": pure[i], "model_place_pure": 0.6, "model_show_pure": 0.8,
            "finishing_position": i + 1, "win_odds": [1.8, 4.5, 9.0][i], "popularity": None,
        })
    # dump の model_win は本番の (α=0.2, γ=1.25) 再構成と一致している前提。
    recon = ms.reconstruct_win(rows, 0.2, 1.25)
    for r, w in zip(rows, recon):
        r["model_win"] = float(w)
    return rows


def test_substitute_replaces_odds_and_reconstructs_blend():
    rows = _rows()
    new = la.substitute(rows, FULL)
    assert [r["win_odds"] for r in new] == [2.0, 4.0, 8.0]
    expected = ms.reconstruct_win([dict(r, win_odds=FULL[r["horse_num"]]) for r in rows], 0.2, 1.25)
    assert [r["model_win"] for r in new] == pytest.approx(list(expected))
    # 元の行は変えない・pure と place/show はそのまま。
    assert [r["win_odds"] for r in rows] == [1.8, 4.5, 9.0]
    assert [r["model_win_pure"] for r in new] == [0.5, 0.3, 0.2]


def test_substitute_with_original_odds_is_identity():
    rows = _rows()
    same = la.substitute(rows, {r["horse_num"]: r["win_odds"] for r in rows})
    assert [r["model_win"] for r in same] == pytest.approx([r["model_win"] for r in rows], abs=1e-12)


def test_classify_mismatches_explains_scratched_horse():
    # 本番 blend が取消馬（馬番 4・dump に無い）の implied を分母に入れた model_win を作り、
    # dump の win_odds からの再構成とは一致しないが、取消馬込みの再構成とは一致することを確かめる。
    from collections import OrderedDict

    rows = _rows()
    race_odds = {1: 1.8, 2: 4.5, 3: 9.0, 4: 6.0}
    prod = la.reconstruct_with_scratched(rows, race_odds)
    for r, w in zip(rows, prod):
        r["model_win"] = float(w)
    assert float(np.max(np.abs(ms.reconstruct_win(rows, 0.2, 1.25) - prod))) > 1e-6  # dump だけでは再現できない
    races = OrderedDict([("R", rows), ("X", _rows("X"))])
    races["X"][0]["model_win"] += 0.01  # 取消馬も無く説明できない不一致
    explained, unexplained = la.classify_mismatches(races, ["R", "X"], {"R": race_odds, "X": {1: 1.8, 2: 4.5, 3: 9.0}})
    assert explained == ["R"] and unexplained == ["X"]


def test_reconstruct_with_scratched_equals_plain_when_no_extra_horse():
    rows = _rows()
    ro = {r["horse_num"]: r["win_odds"] for r in rows}
    assert list(la.reconstruct_with_scratched(rows, ro)) == pytest.approx(list(ms.reconstruct_win(rows, 0.2, 1.25)), abs=1e-12)


def test_select_points_ignores_extra_horses_in_snapshot():
    # スナップショットに出走しない馬番（取消馬 9）が混ざっても、出走馬だけを返す（出走馬だけで正規化する前提）。
    e = _entry("2026-07-04", "15:00", {"09:00": {**FULL, 9: 30.0}, "14:30": FULL, "14:58": FULL})
    assert la.select_points(e, {1, 2, 3})["morning"] == FULL


def test_reconstruct_with_scratched_hand_computed():
    # 2 頭（pure 0.6/0.4）＋取消 1 頭。γ=1.25 なので est ∝ u^0.8。オッズ 2/4・取消 4 → implied 0.5/0.25/0.25。
    # q = (0.5, 0.25)/1.0、mix = 0.2·ê + 0.8·q を正規化して ^1.25 を正規化。手で独立に計算した値と比べる。
    rows = [
        {"horse_num": 1, "model_win_pure": 0.6},
        {"horse_num": 2, "model_win_pure": 0.4},
    ]
    est = np.array([0.6 ** 0.8, 0.4 ** 0.8])
    e = est / est.sum()
    mix = 0.2 * e + 0.8 * np.array([0.5, 0.25])
    mix = mix / mix.sum()
    p = mix ** 1.25
    expected = p / p.sum()
    got = la.reconstruct_with_scratched(rows, {1: 2.0, 2: 4.0, 3: 4.0})
    assert list(got) == pytest.approx(list(expected), abs=1e-12)
    # 取消馬を分母に入れない（出走馬だけの）正規化とは別の値になる。
    plain = la.reconstruct_with_scratched(rows, {1: 2.0, 2: 4.0})
    assert abs(got[0] - plain[0]) > 1e-3


def test_check_fidelity_boundary():
    la.check_fidelity(1, 100)  # ちょうど 1% は通る
    la.check_fidelity(0, 0)
    with pytest.raises(SystemExit):
        la.check_fidelity(2, 100)


def test_evaluate_point_direction_and_names():
    # blended が勝ち馬に確率を寄せ、市場（オッズ）は逆に人気薄を本命にする合成の表。
    # 2 日 × 3 レース。Δ = blended − market が正（的中で上・LogLoss で下）になり、向きが逆でないことを確かめる。
    from collections import OrderedDict

    table = OrderedDict()
    for d, date in enumerate(["2026-07-04", "2026-07-05"]):
        for k in range(3):
            rid = f"R{d}{k}"
            pure = [0.7, 0.2, 0.1]
            odds = [8.0, 2.0, 4.0]  # 市場の本命は馬 2 だが、勝つのは馬 1
            rows = [
                {"race_id": rid, "date": date, "horse_num": i + 1, "model_win": pure[i], "model_place": 0.5,
                 "model_show": 0.8, "model_win_pure": pure[i], "model_place_pure": 0.5, "model_show_pure": 0.8,
                 "finishing_position": i + 1, "win_odds": odds[i], "popularity": None}
                for i in range(3)
            ]
            table[rid] = rows
    r = la.evaluate_point(table, n_boot=50, seed=1)
    assert r["races"] == 6 and r["days"] == 2
    assert r["delta"]["win_hit"][0] == pytest.approx(1.0)  # blended は 6/6、市場は 0/6
    assert r["delta"]["win_logloss"][0] < 0
    assert r["verdict"] == "採用候補"


# ---------- 判定規則（事前コミット） ----------


def test_decide_keeps_alpha_only_when_candidate():
    assert la.decide("採用候補") == "α=0.2 維持"
    assert la.decide("判定不能") == "α=0 に倒す"
    assert la.decide("悪化") == "α=0 に倒す"


# ---------- 母集合（3 時点で同じ集合） ----------


def test_build_point_tables_uses_common_races():
    races = {"A": _rows("A"), "B": _rows("B")}
    snaps = {
        "A": _entry("2026-07-04", "15:00", {"09:00": FULL, "14:30": FULL, "14:58": FULL}),
        "B": _entry("2026-07-04", "15:00", {"09:00": FULL, "10:00": FULL}),  # T-40 が無い
    }
    tables, counts = la.build_point_tables(races, snaps)
    assert set(tables) == {"morning", "t40", "last"}
    for t in tables.values():
        assert list(t) == ["A"]
    assert counts["no_complete_point"] == 1


if __name__ == "__main__":
    sys.exit(pytest.main([__file__, "-q"]))
