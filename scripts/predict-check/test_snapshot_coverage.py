"""snapshot_coverage.py の純関数テスト（pytest 不要・`python3 test_snapshot_coverage.py`）.

DB に触れない hhmm_to_min / fetched_at_to_jst_min / classify / build_coverage / parse_rows の
不変量を固定する。
"""
import snapshot_coverage as C


def test_hhmm_to_min():
    assert C.hhmm_to_min("09:50") == 9 * 60 + 50
    assert C.hhmm_to_min("16:05") == 16 * 60 + 5
    assert C.hhmm_to_min("0:00") == 0
    assert C.hhmm_to_min("24:00") is None  # 時が範囲外
    assert C.hhmm_to_min("09:60") is None  # 分が範囲外
    assert C.hhmm_to_min("") is None


def test_fetched_at_to_jst_min():
    # UTC 05:40 → JST 14:40。UTC 00:50 → JST 09:50。小数秒・TZ サフィックス込みでも時刻部だけ見る。
    assert C.fetched_at_to_jst_min("2026-06-27T05:40:45.19+00:00") == 14 * 60 + 40
    assert C.fetched_at_to_jst_min("2026-06-27T00:50:00+00:00") == 9 * 60 + 50
    # 日跨ぎ（UTC 23:30 → JST 08:30 翌日）も mod 1440 で分は正しい。
    assert C.fetched_at_to_jst_min("2026-06-26T23:30:00+00:00") == 8 * 60 + 30
    # スペース区切り（timestamptz/datestyle 変化で psql が出す形式）も T 同様にパースする。
    assert C.fetched_at_to_jst_min("2026-06-27 05:40:45+00") == 14 * 60 + 40
    assert C.fetched_at_to_jst_min("壊れた") is None
    # UTC 以外のオフセットは二重シフトを避けるため拒否（None→bad_ts で顕在化、沈黙故障にしない）。
    assert C.fetched_at_to_jst_min("2026-06-27T14:40:45+09:00") is None
    assert C.fetched_at_to_jst_min("2026-06-27 14:40:45+09") is None
    # Z 表記の UTC は受理。
    assert C.fetched_at_to_jst_min("2026-06-27T05:40:45Z") == 14 * 60 + 40


def test_classify_bad_ts():
    # snapshot はあるが fetched_at をパース不能 → 'none' でなく 'bad_ts'（取りこぼしと混同しない）。
    ev = C.classify(14 * 60 + 45, None, n_snaps=3, max_lag_min=10)
    assert ev["status"] == "bad_ts" and ev["lag_min"] is None, ev


def test_classify():
    post = 14 * 60 + 45  # 14:45 発走
    # 最終 snapshot が 14:40（5 分前）→ ok（max_lag=10）。
    assert C.classify(post, 14 * 60 + 40, n_snaps=3, max_lag_min=10)["status"] == "ok"
    # 最終 snapshot が 14:00（45 分前）→ gap。
    g = C.classify(post, 14 * 60 + 0, n_snaps=1, max_lag_min=10)
    assert g["status"] == "gap" and g["lag_min"] == 45
    # snapshot 無し → none（lag は None）。
    n = C.classify(post, None, n_snaps=0, max_lag_min=10)
    assert n["status"] == "none" and n["lag_min"] is None
    # 発走以降に取れている（lag 負）→ ok。
    assert C.classify(post, 14 * 60 + 50, n_snaps=2, max_lag_min=10)["status"] == "ok"
    # 境界: ちょうど max_lag は ok（gap は厳密超過）。
    assert C.classify(post, post - 10, n_snaps=1, max_lag_min=10)["status"] == "ok"
    assert C.classify(post, post - 11, n_snaps=1, max_lag_min=10)["status"] == "gap"


def test_build_coverage_filters_and_sorts():
    rows = [
        # 函館2R: 朝 1 本(09:04=UTC00:04)だけ・post 10:20 → 76 分前 gap（実証例の再現）。
        ("R2", "函館", "2", "10:20", "2026-06-27T00:04:00+00:00", 1),
        # 函館1R: 締切直前 09:43(UTC00:43)・post 09:50 → 7 分前 ok。
        ("R1", "函館", "1", "09:50", "2026-06-27T00:43:00+00:00", 6),
        # post_time 空は判定不能で除外。
        ("R3", "函館", "3", "", "2026-06-27T00:00:00+00:00", 2),
        # snapshot 無し → none。
        ("R4", "函館", "4", "11:20", "", 0),
    ]
    cov = C.build_coverage(rows, max_lag_min=10)
    # post_time 空の R3 は落ち、venue/race_num 昇順。
    assert [r["race_num"] for r in cov] == [1, 2, 4], cov
    by_num = {r["race_num"]: r for r in cov}
    assert by_num[1]["status"] == "ok"
    assert by_num[2]["status"] == "gap" and by_num[2]["lag_min"] == 76
    assert by_num[4]["status"] == "none"


def test_parse_rows_column_guard():
    good = "\t".join(["R1", "函館", "1", "09:50", "2026-06-27T00:43:00+00:00", "6"])
    bad_cols = "R1\t函館\t1"            # 列数不足
    bad_n = "\t".join(["R2", "函館", "2", "10:20", "", "x"])      # n_snaps 非数値
    bad_rnum = "\t".join(["R3", "函館", "x", "10:20", "", "2"])   # race_num 非数値
    rows = C.parse_rows("\n".join([good, bad_cols, bad_n, bad_rnum]) + "\n")
    assert len(rows) == 1 and rows[0][0] == "R1" and rows[0][5] == 6, rows


def test_psql_dump_rejects_bad_date():
    # _psql_dump は呼び出し側検証に依存せず日付形式を再検証する（多層防御・subprocess 前に弾く）。
    for bad in ("2026-6-27", "2026-06-27; DROP", "', OR '1'='1"):
        try:
            C._psql_dump("postgres://unused", bad)
        except ValueError:
            continue
        raise AssertionError(f"不正日付 {bad!r} が ValueError にならなかった")


def _run_all():
    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]
    for fn in fns:
        fn()
        print(f"  ok  {fn.__name__}")
    print(f"\n{len(fns)} passed")


if __name__ == "__main__":
    _run_all()
