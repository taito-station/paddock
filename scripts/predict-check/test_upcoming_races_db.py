#!/usr/bin/env python3
"""upcoming_races_db.select_from_rows / valid_date の単体テスト（#237）。

DB・ネットワークに触れない純粋関数だけを検証する。窓判定そのものは upcoming_races の
select_upcoming を再利用しており（test_upcoming_races.py が担保）、本テストは「DB 行 →
post_time NULL 除外 → 窓判定」の DB 経路固有部分を検証する。
実行: `python3 -m pytest test_upcoming_races_db.py` もしくは `python3 test_upcoming_races_db.py`。
"""
import argparse
import contextlib
import io

import pgq
import upcoming_races_db
from upcoming_races import to_minutes
from upcoming_races_db import select_from_rows, valid_date

# (race_id, post_time) の DB 行を模す。post_time が None/空 の行も混ぜる。
ROWS = [
    ("2026-3-tokyo-5-1R", "9:50"),
    ("2026-3-tokyo-5-2R", "10:20"),
    ("2026-3-tokyo-5-3R", "10:55"),
    ("2026-3-tokyo-5-4R", "11:30"),
    ("2026-3-nakayama-6-11R", "16:00"),
]


def test_excludes_finished_races():
    # now=10:30 → 9:50 と 10:20 は発走済みで除外。
    got = select_from_rows(ROWS, to_minutes("10:30"), window_min=60)
    assert got == ["2026-3-tokyo-5-3R", "2026-3-tokyo-5-4R"]


def test_window_upper_bound_excludes_far_future():
    got = select_from_rows(ROWS, to_minutes("9:00"), window_min=60)
    assert got == ["2026-3-tokyo-5-1R"]


def test_skips_rows_without_post_time():
    # post_time が None/空 の行は判定不能として除外する（カード投入済みだが発走時刻未取得）。
    rows = [
        ("2026-3-tokyo-5-1R", "10:10"),
        ("2026-3-tokyo-5-2R", None),
        ("2026-3-tokyo-5-3R", ""),
    ]
    got = select_from_rows(rows, to_minutes("10:00"), window_min=30)
    assert got == ["2026-3-tokyo-5-1R"]


def test_sorted_by_post_time():
    rows = [("late", "11:30"), ("early", "10:55")]
    got = select_from_rows(rows, to_minutes("10:00"), window_min=120)
    assert got == ["early", "late"]


def test_empty_when_all_finished():
    got = select_from_rows(ROWS, to_minutes("17:00"), window_min=30)
    assert got == []


def test_empty_when_no_rows():
    assert select_from_rows([], to_minutes("10:00"), window_min=30) == []


# --- valid_date（date 入力検証） ---

def test_valid_date_accepts_well_formed():
    assert valid_date("2026-06-20") == "2026-06-20"


def test_valid_date_rejects_malformed():
    # 形式不正（ゼロ埋めでない/区切り違い等）と、形式 OK だが暦上存在しない日の両方を弾く。
    for bad in ("20260620", "2026-6-20", "2026/06/20", "abc", "",
                "2026-13-45", "2026-02-30", "2026-00-10"):
        try:
            valid_date(bad)
        except argparse.ArgumentTypeError:
            continue
        raise AssertionError(f"不正値が弾かれていない: {bad!r}")


def test_fetch_rows_failure_forwards_stderr_and_exits_nonzero():
    # prefetch_odds.sh との契約（#714）: 失敗時は psql の stderr を転記して非 0 終了し、stdout には
    # 何も出さない（stdout は race_id 列として読まれる）。pgq.query を失敗させて再現する。
    def failing_query(sql, url=None, variables=None):
        raise pgq.PsqlError("psql 失敗 (exit 2, postgres://u@h/db): refused",
                            stderr="psql: error: connection refused\n")

    orig = pgq.query
    pgq.query = failing_query
    out, err = io.StringIO(), io.StringIO()
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            upcoming_races_db.fetch_rows("2026-09-26", "postgres://u:pw@h/db")
        raise AssertionError("SystemExit が出るはず")
    except SystemExit as e:
        assert e.code not in (0, None)
        assert "pw" not in str(e.code)
    finally:
        pgq.query = orig
    assert out.getvalue() == ""
    assert "connection refused" in err.getvalue()


def test_fetch_rows_failure_before_psql_keeps_reason():
    # psql を起動する前の失敗（psql 不在・URL 形式不正）は stderr が空。理由が終了文言に残ること。
    def failing_query(sql, url=None, variables=None):
        raise pgq.PsqlError("psql が見つかりません（PATH を確認）")

    orig = pgq.query
    pgq.query = failing_query
    try:
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            upcoming_races_db.fetch_rows("2026-09-26", "postgres://h/db")
        raise AssertionError("SystemExit が出るはず")
    except SystemExit as e:
        assert "psql が見つかりません" in str(e.code), e.code
    finally:
        pgq.query = orig


def test_fetch_rows_maps_cells_to_pairs():
    orig = pgq.query
    pgq.query = lambda sql, url=None, variables=None: [["2026-3-tokyo-5-1R", "9:50"]]
    try:
        assert upcoming_races_db.fetch_rows("2026-09-26", "postgres://h/db") == [
            ("2026-3-tokyo-5-1R", "9:50")]
    finally:
        pgq.query = orig


if __name__ == "__main__":
    import sys

    failed = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            try:
                fn()
                print(f"ok   {name}")
            except Exception as e:
                failed += 1
                print(f"FAIL {name}: {type(e).__name__}: {e}")
    sys.exit(1 if failed else 0)
