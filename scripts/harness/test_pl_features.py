"""pl_features.py のユニットテスト（#720）。

対象: 入力列の許可リスト（市場情報の混入拒否）/ 抽出 SQL の静的検査 / as-of（当日以降の結果を使わない）/
市場列への不変性 / 適性・フォーム・騎手成績の既知値 / 出走馬の絞り込み。
"""

import os
import re
import sys

try:
    import numpy as np
    import pytest
except ImportError:
    if __name__ == "__main__":
        print("skip: numpy/pytest 不在（ローカルは pytest で実行）")
        sys.exit(0)
    raise

import pl_features as pf

HERE = os.path.dirname(os.path.abspath(__file__))


def _row(race_id, date, name, pos, *, venue="東京", surface="turf", distance=1600, going="良", num=1,
         jockey="J1", trainer="T1", time=None, status="finished", field=None):
    return {
        "race_id": race_id, "date": date, "venue": venue, "surface": surface, "distance": distance,
        "track_condition": going, "horse_num": num, "gate_num": num, "horse_name": name,
        "jockey": jockey, "trainer": trainer, "weight_carried": 56.0, "horse_weight": 480,
        "weight_change": 0, "status": status, "finishing_position": pos,
        "time_seconds": time if time is not None else (96.0 + (pos or 9)),
    }


def _race(race_id, date, names, **kw):
    """names の順に 1 着・2 着…。"""
    return [_row(race_id, date, n, i + 1, num=i + 1, **kw) for i, n in enumerate(names)]


# ---------- 入力の許可リスト（市場非依存） ----------


def test_allowed_columns_exclude_market_information():
    forbidden = {"odds", "popularity", "win_odds", "place_odds"}
    assert not forbidden & set(pf.ALLOWED_COLUMNS)


def test_extract_sql_selects_only_allowed_columns():
    sql = open(os.path.join(HERE, "pl_features_extract.sql"), encoding="utf-8").read()
    body = re.sub(r"--[^\n]*", "", sql)  # コメントの説明文は検査しない
    # 市場情報（接頭辞付きの列名も含む）・ワイルドカード・書き込み文・複文を含めない
    assert not re.search(r"odds|popularity", body, re.I)
    assert "*" not in body and ";" not in body
    assert not re.search(r"\b(insert|update|delete|returning|drop|alter|create|truncate)\b", body, re.I)
    # SELECT 句の各式は「表別名.許可列」か「COALESCE(表別名.許可列, '') AS 許可列」だけ（別名で市場列を
    # 許可列名に化けさせない）。出力列の並びも許可リストと一致させる
    # 読む表は results と races だけ（FROM 以降を固定。サブクエリ・別の表で市場由来の列を載せない）
    assert len(re.findall(r"\bSELECT\b", body, re.I)) == 1
    tail = " ".join(body[re.search(r"\bFROM\b", body).start():].split())
    assert tail == "FROM results x JOIN races r USING (race_id) ORDER BY r.date, x.race_id, x.horse_num"
    select = re.search(r"SELECT(.*?)FROM", body, re.S | re.I).group(1)
    items = [i.strip() for i in select.split(",\n")]
    names = []
    for it in items:
        m = re.fullmatch(r"[xr]\.(\w+)", it) or re.fullmatch(r"COALESCE\([xr]\.(\w+), ''\) AS (\w+)", it)
        assert m, it
        assert m.group(1) == m.groups()[-1], it
        names.append(m.group(1))
    assert names == pf.ALLOWED_COLUMNS


def test_load_raw_rejects_extra_or_missing_columns(tmp_path):
    ok = "\t".join(pf.ALLOWED_COLUMNS)
    p = tmp_path / "raw.tsv"
    p.write_text(ok + "\todds\n", encoding="utf-8")
    with pytest.raises(ValueError, match="許可"):
        pf.load_raw(str(p))
    p.write_text("\t".join(pf.ALLOWED_COLUMNS[:-1]) + "\n", encoding="utf-8")
    with pytest.raises(ValueError, match="許可"):
        pf.load_raw(str(p))


def test_load_raw_parses_types_and_nulls(tmp_path):
    vals = {c: "" for c in pf.ALLOWED_COLUMNS}
    vals.update(race_id="R1", date="2025-03-01", venue="東京", surface="turf", distance="1600",
                horse_num="3", gate_num="2", horse_name=" ホース ", status="did_not_finish",
                weight_carried="55.5")
    p = tmp_path / "raw.tsv"
    p.write_text("\t".join(pf.ALLOWED_COLUMNS) + "\n" + "\t".join(vals[c] for c in pf.ALLOWED_COLUMNS) + "\n",
                 encoding="utf-8")
    (r,) = pf.load_raw(str(p))
    assert r["distance"] == 1600 and r["horse_num"] == 3 and r["weight_carried"] == 55.5
    assert r["finishing_position"] is None and r["time_seconds"] is None and r["horse_weight"] is None
    assert r["horse_name"] == "ホース"


def test_load_raw_rejects_bad_keys_status_and_missing_labels(tmp_path):
    base = {c: "" for c in pf.ALLOWED_COLUMNS}
    base.update(race_id="R1", date="2025-03-01", venue="東京", surface="turf", distance="1600",
                horse_num="3", gate_num="2", horse_name="A", status="finished", finishing_position="1")
    for i, (patch, msg) in enumerate((({"horse_name": ""}, "horse_name が空"), ({"race_id": ""}, "race_id が空"),
                                      ({"status": "finish"}, "status が未知"),
                                      ({"finishing_position": ""}, "finishing_position が空"))):
        vals = dict(base, **patch)
        p = tmp_path / f"bad{i}.tsv"
        p.write_text("\t".join(pf.ALLOWED_COLUMNS) + "\n" + "\t".join(vals[c] for c in pf.ALLOWED_COLUMNS) + "\n",
                     encoding="utf-8")
        with pytest.raises(ValueError, match=msg):
            pf.load_raw(str(p))


def test_load_raw_rejects_non_finite_and_bad_values(tmp_path):
    base = {c: "" for c in pf.ALLOWED_COLUMNS}
    base.update(race_id="R1", date="2025-03-01", venue="東京", surface="turf", distance="1600",
                horse_num="3", gate_num="2", horse_name="A", status="finished")
    for col, val in (("time_seconds", "NaN"), ("weight_carried", "inf"), ("distance", "x"), ("date", "2025/03/01")):
        vals = dict(base, **{col: val})
        p = tmp_path / f"{col}.tsv"
        p.write_text("\t".join(pf.ALLOWED_COLUMNS) + "\n" + "\t".join(vals[c] for c in pf.ALLOWED_COLUMNS) + "\n",
                     encoding="utf-8")
        with pytest.raises(ValueError, match=rf"{col}.tsv:2 {col}"):
            pf.load_raw(str(p))


# ---------- 出走馬・as-of ----------


def test_only_starters_are_emitted():
    rows = _race("R1", "2025-03-01", ["A", "B", "C"])
    rows.append(_row("R1", "2025-03-01", "X", None, num=4, status="scratched"))
    rows.append(_row("R1", "2025-03-01", "Y", None, num=5, status="did_not_finish"))
    out = pf.build_features(rows)
    names = {o["horse_name"] for o in out}
    assert names == {"A", "B", "C", "Y"}  # 取消・除外は出走していない。中止は出走している


def test_features_use_only_earlier_dates():
    past = _race("R1", "2025-03-01", ["A", "B", "C", "D"])
    today = _race("R2", "2025-04-01", ["D", "C", "B", "A"])
    base = {o["horse_name"]: o for o in pf.build_features(past + today) if o["race_id"] == "R2"}
    # 当日・未来のレースの結果を書き換えても R2 の特徴量は変わらない
    future = _race("R3", "2025-05-01", ["A", "B", "C", "D"])
    for r in today:
        r["finishing_position"] = 5 - r["finishing_position"]
    after = {o["horse_name"]: o for o in pf.build_features(past + today + future) if o["race_id"] == "R2"}
    for n in base:
        for f in pf.FEATURES:
            assert base[n][f] == pytest.approx(after[n][f]), (n, f)
    # 同じ日の別レース（同じ騎手）の結果も使わない。R2 より先に処理される race_id にして、
    # 「レースごとに即時に履歴へ足す」実装なら R2 の騎手成績が変わるようにする
    same_day = _race("R0", "2025-04-01", ["E", "F", "G"], jockey="J1")
    again = {o["horse_name"]: o for o in pf.build_features(past + today + same_day) if o["race_id"] == "R2"}
    assert again["A"]["jockey_win"] == pytest.approx(base["A"]["jockey_win"])


def test_invariant_to_market_columns():
    # 補助の検査: build_features が市場列を読まないこと。市場非依存を担保する主な仕組みは、入口の
    # 許可リスト（test_load_raw_rejects_extra_or_missing_columns）と抽出 SQL の静的検査
    rows = _race("R1", "2025-03-01", ["A", "B", "C", "D"]) + _race("R2", "2025-04-01", ["B", "A", "D", "C"])
    base = pf.build_features([dict(r) for r in rows])
    rng = np.random.default_rng(0)
    noisy = [dict(r, odds=float(rng.uniform(1, 100)), popularity=int(rng.integers(1, 18))) for r in rows]
    for a, b in zip(base, pf.build_features(noisy)):
        assert [a[f] for f in pf.FEATURES] == [b[f] for f in pf.FEATURES]


# ---------- 既知値 ----------


def test_history_count_form_and_first_timer():
    rows = (_race("R1", "2025-03-01", ["A", "B", "C"])
            + _race("R2", "2025-03-15", ["B", "A", "C"])
            + _race("R3", "2025-04-01", ["A", "B", "C", "N"]))
    f = {o["horse_name"]: o for o in pf.build_features(rows) if o["race_id"] == "R3"}
    assert f["A"]["n_runs"] == pytest.approx(np.log1p(2))
    # A の相対着順: R1 1 着 → 0.0、R2 2 着 → (2-1)/(3-1) = 0.5
    assert f["A"]["form3"] == pytest.approx(0.25)
    assert f["A"]["last_rel"] == pytest.approx(0.5)
    assert f["A"]["days_since"] == pytest.approx(np.log1p(17))
    assert f["A"]["no_hist"] == 0.0
    assert f["N"]["no_hist"] == 1.0 and f["N"]["n_runs"] == 0.0
    assert f["N"]["form3"] == pytest.approx(pf.NEUTRAL_REL)


def test_surface_aptitude_sign():
    # A は芝で好走・ダートで凡走。ダートのレースでは芝の成績に引っ張られず、ダート適性が負に出る
    rows = []
    for i, d in enumerate(["2025-03-01", "2025-03-08", "2025-03-15"]):
        rows += _race(f"T{i}", d, ["A", "B", "C", "D"], surface="turf")
    for i, d in enumerate(["2025-03-22", "2025-03-29"]):
        rows += _race(f"D{i}", d, ["B", "C", "D", "A"], surface="dirt")
    rows += _race("X", "2025-04-05", ["A", "B", "C", "D"], surface="dirt")
    rows += _race("Y", "2025-04-05", ["A", "B", "C", "D"], surface="turf", venue="中山")
    f = {(o["race_id"], o["horse_name"]): o for o in pf.build_features(rows)}
    # 相対着順は小さいほど良いので、適性 =「全体 − 条件（全体へ縮約）」で正が得意
    assert f[("X", "A")]["surface_apt"] < 0 < f[("Y", "A")]["surface_apt"]
    # 既知値: 全体 0.4（0,0,0,1,1）・ダート 2 走とも 1.0 → (2·1.0 + m·0.4)/(2 + m)
    m = pf.APT_M
    assert f[("X", "A")]["surface_apt"] == pytest.approx(0.4 - (2 * 1.0 + m * 0.4) / (2 + m))


def test_distance_gap_and_direction():
    rows = (_race("R1", "2025-03-01", ["A", "B"], distance=1200)
            + _race("R2", "2025-03-08", ["A", "B"], distance=1400)
            + _race("R3", "2025-04-01", ["A", "B", "C"], distance=2000))
    f = {o["horse_name"]: o for o in pf.build_features(rows) if o["race_id"] == "R3"}
    assert f["A"]["dist_dir"] == pytest.approx((2000 - 1300) / 1000)
    assert f["A"]["dist_gap"] == pytest.approx(0.7)
    assert f["C"]["dist_dir"] == 0.0 and f["C"]["dist_gap"] == 0.0


def test_jockey_rates_are_shrunk_as_of():
    rows = _race("R1", "2025-03-01", ["A", "B"], jockey="J1") + _race("R2", "2025-04-01", ["C", "D"], jockey="J1")
    f = {o["horse_name"]: o for o in pf.build_features(rows) if o["race_id"] == "R2"}
    # J1 は R1 で 2 騎乗 1 勝。縮約: (1 + m·prior) / (2 + m)
    m, prior = pf.JOCKEY_M, pf.WIN_PRIOR
    assert f["C"]["jockey_win"] == pytest.approx((1 + m * prior) / (2 + m))
    r1 = {o["horse_name"]: o for o in pf.build_features(rows) if o["race_id"] == "R1"}
    assert r1["A"]["jockey_win"] == pytest.approx(prior)  # 履歴なしは事前分布そのもの


def test_speed_uses_par_from_before_the_month():
    # 基準タイム（コース × 馬場の平均・SD）は予測月の月初より前の完走だけから作る
    rows = []
    for i in range(pf.PAR_MIN_N):
        rows += [_row(f"P{i}", "2025-02-01", f"H{i}", 1, time=96.0 + (i % 5) * 0.5, num=1)]
    rows += _race("R1", "2025-03-05", ["A", "B"], time=None)
    rows += _race("R2", "2025-03-20", ["A", "B"])  # R1 と同月: R1 の par は 2 月まで
    f = {(o["race_id"], o["horse_name"]): o for o in pf.build_features(rows)}
    assert f[("R2", "A")]["no_speed"] == 0.0
    assert f[("R1", "A")]["no_speed"] == 1.0  # 過去走なし
    # 既知値: 基準は 2 月の 30 走だけ（同月の R1 の 97.0・98.0 を含めない）。A の R1 は 1 着 97.0 秒
    ts = np.array([96.0 + (i % 5) * 0.5 for i in range(pf.PAR_MIN_N)])
    assert f[("R2", "A")]["best_speed3"] == pytest.approx((ts.mean() - 97.0) / ts.std())


def test_dnf_counts_as_last_and_jockey_top3():
    rows = _race("R1", "2025-03-01", ["A", "B", "C", "D"], jockey="J1")
    rows[3]["finishing_position"] = None  # D は中止
    rows[3]["status"] = "did_not_finish"
    rows += _race("R2", "2025-04-01", ["D", "E"], jockey="J1")
    f = {o["horse_name"]: o for o in pf.build_features(rows) if o["race_id"] == "R2"}
    assert f["D"]["last_rel"] == 1.0 and f["D"]["dnf_rate"] == 1.0  # 中止は最下位扱い
    # J1 は R1 で 4 騎乗・3 着内 3（1〜3 着）
    assert f["E"]["jockey_top3"] == pytest.approx((3 + pf.JOCKEY_M * pf.TOP3_PRIOR) / (4 + pf.JOCKEY_M))


def test_speed_is_capped():
    rows = [_row(f"P{i}", "2025-02-01", f"H{i}", 1, time=96.0 + (i % 5) * 0.5) for i in range(pf.PAR_MIN_N)]
    rows += [_row("R1", "2025-03-05", "A", 1, time=200.0)]  # 大差の失速（または記録誤り）
    rows += _race("R2", "2025-03-20", ["A", "B"])
    f = {(o["race_id"], o["horse_name"]): o for o in pf.build_features(rows)}
    assert f[("R2", "A")]["best_speed3"] == -pf.SPEED_CAP


def test_features_are_finite():
    rows = _race("R1", "2025-03-01", ["A", "B", "C"]) + _race("R2", "2025-04-01", ["A", "B", "C"])
    rows[0]["horse_weight"] = None
    rows[1]["weight_change"] = None
    for o in pf.build_features(rows):
        assert all(np.isfinite(o[f]) for f in pf.FEATURES)


# ---------- 馬場不明（#745: 空の馬場を良として数えない） ----------


def _going_history(last_going):
    """A は良で 1 着・重で最下位。最後の走の馬場を last_going にする。"""
    return (_race("G1", "2025-03-01", ["A", "B", "C"], going="良")
            + _race("G2", "2025-03-08", ["B", "C", "A"], going="重")
            + _race("G3", "2025-03-15", ["B", "C", "A"], going=last_going))


def test_unknown_going_today_gives_neutral_going_apt():
    # 過去走にも馬場不明（G3）を置く: 不明どうしを同じ馬場として比べない
    rows = _going_history("") + _race("X", "2025-04-05", ["A", "B", "C"], going="")
    f = {(o["race_id"], o["horse_name"]): o for o in pf.build_features(rows)}
    assert f[("X", "A")]["going_apt"] == 0.0  # 当日の馬場が分からないので適性は中立（データなしと同じ）


def test_unknown_going_history_is_not_counted_as_good():
    # G3（馬場不明・最下位）を良の走として数えると、良の適性が下がる
    rows = _going_history("") + _race("X", "2025-04-05", ["A", "B", "C"], going="良")
    f = {(o["race_id"], o["horse_name"]): o for o in pf.build_features(rows)}
    m = pf.APT_M
    overall = (0.0 + 1.0 + 1.0) / 3  # 全体成績には馬場不明の走も入る
    assert f[("X", "A")]["going_apt"] == pytest.approx(overall - (0.0 + m * overall) / (1 + m))


def test_unknown_going_times_are_not_in_good_par():
    # 良の 30 走（96〜98 秒）と馬場不明の 30 走（110 秒）。良の基準に不明の 110 秒を混ぜない
    good = [96.0 + (i % 5) * 0.5 for i in range(pf.PAR_MIN_N)]
    rows = [_row(f"P{i}", "2025-02-01", f"H{i}", 1, time=t, going="良") for i, t in enumerate(good)]
    # 同じ日に置く（馬場不明の走と良の走が同日に並んでも基準タイムの集計が落ちないこと）
    rows += [_row(f"U{i}", "2025-02-01", f"K{i}", 1, time=110.0, going="") for i in range(pf.PAR_MIN_N)]
    rows += [_row("R1", "2025-03-05", "A", 1, time=96.5, going="良"),  # A は良で 96.5 秒
             _row("R1", "2025-03-05", "B", 2, time=97.5, going="良", num=2)]
    rows += _race("R2", "2025-03-20", ["A", "B"])
    f = {(o["race_id"], o["horse_name"]): o for o in pf.build_features(rows)}
    ts = np.array(good)
    assert f[("R2", "A")]["best_speed3"] == pytest.approx((ts.mean() - 96.5) / ts.std())


def test_unknown_going_run_uses_course_par():
    # 馬場不明の走は良・重どちらの基準でもなく、馬場を区別しないコースの基準で測る
    good = [96.0 + (i % 5) * 0.5 for i in range(pf.PAR_MIN_N)]
    rows = [_row(f"P{i}", "2025-02-01", f"H{i}", 1, time=t, going="良") for i, t in enumerate(good)]
    rows += [_row(f"Q{i}", "2025-02-02", f"K{i}", 1, time=t + 3.0, going="重") for i, t in enumerate(good)]
    # 別コースの馬場不明 30 走（120 秒）。馬場不明どうしを 1 つの基準にまとめない
    rows += [_row(f"U{i}", "2025-02-03", f"M{i}", 1, time=120.0 + (i % 5) * 0.5, going="", venue="中山")
             for i in range(pf.PAR_MIN_N)]
    rows += [_row("R1", "2025-03-05", "A", 1, time=97.0, going=""),
             _row("R1", "2025-03-05", "B", 2, time=98.0, going="", num=2)]
    rows += _race("R2", "2025-03-20", ["A", "B"])
    f = {(o["race_id"], o["horse_name"]): o for o in pf.build_features(rows)}
    ts = np.array(good + [t + 3.0 for t in good])
    assert f[("R2", "A")]["best_speed3"] == pytest.approx((ts.mean() - 97.0) / ts.std())


if __name__ == "__main__":
    sys.exit(pytest.main([__file__, "-q"]))
