#!/usr/bin/env python3
"""odds_guard（netkeiba の未発売番兵）の回帰テスト（#621）。

この壊れ方は**例外を出さない**。番兵をオッズとして受け入れると EV が静かに 3 桁になり、
参考 ROI が 600% を超える（実測）。数字が出ている以上「動いている」ように見えるので、
ここで値そのものを固定する。
"""

import os
import sys

import odds_guard as G

HERE = os.path.dirname(os.path.abspath(__file__))


def test_sentinels_are_loaded_from_the_shared_golden():
    # 正本は Rust と共有のファイル。Rust 側は同じファイルを include_str! して
    # NETKEIBA_SENTINELS と突き合わせている（片方だけ変えればどちらかが落ちる）。
    assert G.SENTINELS_PATH.endswith(
        os.path.join("src", "domain", "src", "odds", "testdata", "netkeiba_sentinels.txt")
    ), G.SENTINELS_PATH
    assert os.path.exists(G.SENTINELS_PATH), G.SENTINELS_PATH
    assert G.NETKEIBA_SENTINELS == (9999.9, 99999.9, 999999.9), G.NETKEIBA_SENTINELS


def test_is_sentinel_matches_every_known_placeholder():
    for s in G.NETKEIBA_SENTINELS:
        assert G.is_sentinel(s), s
        assert not G.is_payout_odds(s), s
    # 文字列で来ても判定できる（TSV 経由の入力）。
    assert G.is_sentinel("99999.9")
    assert not G.is_payout_odds("99999.9")


def test_legitimate_long_shots_are_kept():
    # 三連単には実在する高配当。上限方式を採らない理由そのものなので、通ることを固定する。
    for odds in (111971.9, 200886.6, 99998.9, 100000.0, 999999.8, 2083.5):
        assert not G.is_sentinel(odds), odds
        assert G.is_payout_odds(odds), odds


def test_out_of_range_is_rejected():
    # 下限側は Rust の OddsValue と同じ扱い（有限・1.0 以上）。
    for bad in (0.0, 0.9, -1.0, float("nan"), float("inf"), float("-inf"), "", None, "---.-"):
        assert not G.is_payout_odds(bad), bad
    assert G.is_payout_odds(1.0)


def test_parse_exotic_drops_sentinel_rows():
    # 実際の入口（live_ev.parse_exotic）で落ちることまで見る。TSV 1 行が
    # `pid<TAB>kind<TAB>combo<TAB>odds`。
    import tempfile

    import live_ev as L

    body = (
        "p1\ttrio\t3-7-15\t99999.9\n"   # 未発売 → 落ちる
        "p1\ttrio\t1-2-3\t42.5\n"       # 正常
        "p1\tquinella\t1-2\t99999.9\n"  # 未発売 → 落ちる
        "p1\tquinella\t3-4\t12.3\n"     # 正常
    )
    fd, path = tempfile.mkstemp(suffix=".tsv")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(body)
        qn, tr = L.parse_exotic(path)
        assert tr["p1"] == {(1, 2, 3): 42.5}, tr
        assert qn["p1"] == {(3, 4): 12.3}, qn
    finally:
        os.unlink(path)


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")


if __name__ == "__main__":
    main()
