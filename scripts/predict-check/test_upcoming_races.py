#!/usr/bin/env python3
"""upcoming_races.select_upcoming / to_minutes の単体テスト（#197）。

ネットワークに触れない純粋関数だけを検証する。実行: `python3 -m pytest test_upcoming_races.py`
もしくは `python3 test_upcoming_races.py`（pytest 不在時の素朴ランナー）。
"""
from upcoming_races import select_upcoming, to_minutes

# race_id は 12 桁である必要は無い（select_upcoming はキーを不透明に扱う）が、
# 現実に近い形で並べ替え順も確認する。
POSTS = {
    "r0950": "9:50",
    "r1020": "10:20",
    "r1055": "10:55",
    "r1130": "11:30",
    "r1600": "16:00",
}


def test_to_minutes():
    assert to_minutes("0:00") == 0
    assert to_minutes("9:50") == 9 * 60 + 50
    assert to_minutes("16:00") == 960


def test_excludes_finished_races():
    # now=10:30 → 9:50 と 10:20 は発走済みで除外。
    got = select_upcoming(POSTS, to_minutes("10:30"), window_min=60)
    assert got == ["r1055", "r1130"]


def test_window_upper_bound_excludes_far_future():
    # now=9:00, window=60 → 9:00〜10:00 のみ。10:20 以降は窓の外。
    got = select_upcoming(POSTS, to_minutes("9:00"), window_min=60)
    assert got == ["r0950"]


def test_boundaries_are_inclusive():
    # post == now（ちょうど発走）と post == now+window（窓の端）の両端を含む。
    posts = {"now": "10:00", "edge": "11:00", "over": "11:01"}
    got = select_upcoming(posts, to_minutes("10:00"), window_min=60)
    assert got == ["now", "edge"]


def test_sorted_by_post_time():
    # 入力順に依らず発走時刻昇順で返す。
    posts = {"late": "11:30", "early": "10:55"}
    got = select_upcoming(posts, to_minutes("10:00"), window_min=120)
    assert got == ["early", "late"]


def test_empty_when_all_finished():
    got = select_upcoming(POSTS, to_minutes("17:00"), window_min=60)
    assert got == []


if __name__ == "__main__":
    import sys

    failed = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            try:
                fn()
                print(f"ok   {name}")
            except AssertionError as e:
                failed += 1
                print(f"FAIL {name}: {e}")
    sys.exit(1 if failed else 0)
