#!/usr/bin/env python3
"""upcoming_races.select_upcoming / to_minutes の単体テスト（#197）。

ネットワークに触れない純粋関数だけを検証する。実行: `python3 -m pytest test_upcoming_races.py`
もしくは `python3 test_upcoming_races.py`（pytest 不在時の素朴ランナー）。
"""
import argparse

from nk import parse_post_times
from upcoming_races import select_upcoming, to_minutes, valid_hhmm

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


# --- valid_hhmm（--at 入力検証） ---

def test_valid_hhmm_accepts_well_formed():
    for s in ("0:00", "09:50", "9:50", "23:59", "00:00"):
        assert valid_hhmm(s) == s


def test_valid_hhmm_rejects_out_of_range_and_malformed():
    for bad in ("25:00", "10:99", "24:00", "10", "abc", "9:5", ":30", "10:60"):
        try:
            valid_hhmm(bad)
        except argparse.ArgumentTypeError:
            continue
        raise AssertionError(f"不正値が弾かれていない: {bad!r}")


# --- parse_post_times（race_list HTML パース） ---

# netkeiba race_list_sub.html を模した最小フィクスチャ。各レース項目は
# `class="RaceList_DataItem ..."` で始まり、shutuba リンクの race_id と Itemtime を 1 対 1 で持つ。
FIXTURE = """
<div class="RaceList_Box">
<li class="RaceList_DataItem ">
  <a href="../race/shutuba.html?race_id=202602010401&rf=race_list">
  <div class="Race_Num"><span>1R</span></div></a>
  <span class="RaceList_Itemtime">9:50</span>
</li>
<li class="RaceList_DataItem ">
  <a href="../race/shutuba.html?race_id=202605030611&rf=race_list">
  <div class="Race_Num"><span>11R</span></div></a>
  <span class="RaceList_Itemtime">15:45</span>
</li>
<li class="RaceList_DataItem NoTime">
  <a href="../race/shutuba.html?race_id=202609030699&rf=race_list">特殊行（発走時刻なし）</a>
</li>
</div>
"""


def test_parse_post_times_pairs_id_and_time():
    got = parse_post_times(FIXTURE)
    # 発走時刻 span を持つ 2 レースだけ採用、span 無しの特殊行はスキップ。
    assert got == {"202602010401": "9:50", "202605030611": "15:45"}


def test_parse_post_times_venue_filter():
    # 場コード 05（東京）だけに絞る。race_id の 5-6 桁目が場コード。
    got = parse_post_times(FIXTURE, venue_codes=["05"])
    assert got == {"202605030611": "15:45"}


def test_parse_post_times_empty_html():
    assert parse_post_times("<html>no races</html>") == {}


if __name__ == "__main__":
    import sys

    failed = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            try:
                fn()
                print(f"ok   {name}")
            except Exception as e:  # AssertionError 以外（ValueError 等）も FAIL 計上する
                failed += 1
                print(f"FAIL {name}: {type(e).__name__}: {e}")
    sys.exit(1 if failed else 0)
