#!/usr/bin/env python3
"""upcoming_races_db.select_from_rows / valid_date の単体テスト（#237）。

DB・ネットワークに触れない純粋関数だけを検証する。窓判定そのものは upcoming_races の
select_upcoming を再利用しており（test_upcoming_races.py が担保）、本テストは「DB 行 →
post_time NULL 除外 → 窓判定」の DB 経路固有部分を検証する。
実行: `python3 -m pytest test_upcoming_races_db.py` もしくは `python3 test_upcoming_races_db.py`。
"""
import argparse

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
    for bad in ("20260620", "2026-6-20", "2026/06/20", "abc", ""):
        try:
            valid_date(bad)
        except argparse.ArgumentTypeError:
            continue
        raise AssertionError(f"不正値が弾かれていない: {bad!r}")


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
