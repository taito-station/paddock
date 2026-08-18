#!/usr/bin/env python3
"""odds_guard（netkeiba の未発売番兵）の回帰テスト（#621）。

この壊れ方は**例外を出さない**。番兵をオッズとして受け入れると EV が静かに 3 桁になり、
参考 ROI が 600% を超える（実測）。数字が出ている以上「動いている」ように見えるので、
ここで値そのものを固定する。
"""

import os

import odds_guard as G


def test_sentinels_are_loaded_from_the_shared_golden():
    # 正本は Rust と共有のファイル。Rust 側は同じファイルを include_str! して
    # NETKEIBA_SENTINELS と突き合わせている（片方だけ変えればどちらかが落ちる）。
    assert G.SENTINELS_PATH.endswith(
        os.path.join("src", "domain", "src", "odds", "netkeiba_sentinels.txt")
    ), G.SENTINELS_PATH
    assert os.path.exists(G.SENTINELS_PATH), G.SENTINELS_PATH
    # 番兵は券種別（#630）。win / place は行そのものが無い＝番兵なし（#634）。
    assert G.NETKEIBA_SENTINELS == {
        "wide": (9999.9,),
        "quinella": (99999.9,),
        "exacta": (99999.9,),
        "trio": (99999.9,),
        "trifecta": (999999.9,),
    }, G.NETKEIBA_SENTINELS
    assert "win" not in G.NETKEIBA_SENTINELS
    assert "place" not in G.NETKEIBA_SENTINELS


def test_is_sentinel_matches_every_known_placeholder():
    for bt, values in G.NETKEIBA_SENTINELS.items():
        for s in values:
            assert G.is_sentinel(bt, s), (bt, s)
            assert not G.is_payout_odds(bt, s), (bt, s)
    # 文字列で来ても判定できる（TSV 経由の入力）。
    assert G.is_sentinel("quinella", "99999.9")
    assert not G.is_payout_odds("quinella", "99999.9")


def test_sentinel_scope_is_per_bet_type():
    # #630 の核: 同じ値でも券種が違えば意味が違う。
    # ワイドの 9999.9 は番兵、三連複の 9999.9 は正当な配当（9000〜11000 帯に 6,244 行実在=2026-08-18 実測）。
    assert G.is_sentinel("wide", 9999.9)
    assert not G.is_sentinel("trio", 9999.9)
    assert G.is_payout_odds("trio", 9999.9)
    # 三連単の 99999.9 も正当（番兵は 999999.9 のみ）。
    assert G.is_payout_odds("trifecta", 99999.9)
    assert G.is_sentinel("trifecta", 999999.9)
    # 単勝・複勝に番兵は無い（#634 実測 0 行）。他券種の番兵値でも正当なオッズとして通る。
    for v in (9999.9, 99999.9, 999999.9):
        assert not G.is_sentinel("win", v), v
        assert G.is_payout_odds("win", v), v
        assert not G.is_sentinel("place", v), v
        assert G.is_payout_odds("place", v), v


def test_unknown_bet_type_raises():
    # 未知ラベルを False に畳むと「typo で番兵が素通り」が #621 と同じ静かな壊れ方になる。
    for fn in (G.is_sentinel, G.is_payout_odds):
        for label in ("tansho", "", None, "WIN "):
            try:
                fn(label, 1.0)
                raise AssertionError(f"{fn.__name__}({label!r}) が ValueError を出さなかった")
            except ValueError:
                pass
        # ラベル検証はオッズ値の float 化より先（値が壊れていてもラベルの誤りを検出する）。
        try:
            fn("unknown", "not-a-number")
            raise AssertionError(f"{fn.__name__} が非数値オッズで ValueError を出さなかった")
        except ValueError:
            pass


def test_legitimate_long_shots_are_kept():
    # 三連単には実在する高配当。上限方式を採らない理由そのものなので、通ることを固定する。
    for odds in (111971.9, 200886.6, 99998.9, 100000.0, 999999.8, 2083.5):
        assert not G.is_sentinel("trifecta", odds), odds
        assert G.is_payout_odds("trifecta", odds), odds


def test_out_of_range_is_rejected():
    # 下限側は Rust の OddsValue と同じ扱い（有限・1.0 以上）。券種に依らない。
    for bad in (0.0, 0.9, -1.0, float("nan"), float("inf"), float("-inf"), "", None, "---.-"):
        assert not G.is_payout_odds("win", bad), bad
        assert not G.is_payout_odds("trio", bad), bad
    assert G.is_payout_odds("win", 1.0)


def test_parse_exotic_drops_sentinel_rows():
    # 実際の入口（live_ev.parse_exotic）で落ちることまで見る。TSV 1 行が
    # `pid<TAB>kind<TAB>combo<TAB>odds`。
    import tempfile

    import live_ev as L

    body = (
        "p1\ttrio\t3-7-15\t99999.9\n"   # 未発売 → 落ちる
        "p1\ttrio\t1-2-3\t42.5\n"       # 正常
        "p1\ttrio\t2-4-9\t9999.9\n"     # trio の 9999.9 は正当な配当（#630）→ 残る
        "p1\tquinella\t1-2\t99999.9\n"  # 未発売 → 落ちる
        "p1\tquinella\t3-4\t12.3\n"     # 正常
    )
    fd, path = tempfile.mkstemp(suffix=".tsv")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(body)
        qn, tr = L.parse_exotic(path)
        assert tr["p1"] == {(1, 2, 3): 42.5, (2, 4, 9): 9999.9}, tr
        assert qn["p1"] == {(3, 4): 12.3}, qn
    finally:
        os.unlink(path)


def test_gate_calibration_drops_band_sentinels():
    # band（wide）は**中点化の前**に見ないと、番兵と実値の平均になって検知できない。
    # PR 自身がコメントで難所と明言している箇所なので固定する。
    import gate_calibration as GC

    # 列は rid, bet_type, key, odds, odds_high, fetched の 6 つ（タブ区切り）。
    rows = "\n".join([
        "r1\twide\t1-2\t9999.9\t0.0\t2026-08-15T10:00:00Z",     # 番兵 low → 落ちる
        "r1\twide\t1-3\t9999.9\t9999.9\t2026-08-15T10:00:00Z",  # 両端番兵 → 落ちる（中点でも 9999.9）
        "r1\twide\t2-3\t3.1\t9999.9\t2026-08-15T10:00:00Z",     # high だけ番兵（ワイドは 9999.9）→ 落ちる
        "r1\twide\t3-4\t3.1\t4.9\t2026-08-15T10:00:00Z",        # 正常 → 残る
        "r1\ttrio\t1-2-3\t99999.9\t\t2026-08-15T10:00:00Z",     # scalar 番兵 → 落ちる
        "r1\ttrio\t2-3-4\t42.5\t\t2026-08-15T10:00:00Z",        # 正常 → 残る
        "r1\ttrio\t3-4-5\t9999.9\t\t2026-08-15T10:00:00Z",      # trio の 9999.9 は正当（#630）→ 残る
    ])
    got = GC.load_odds(rows)
    at = got["r1"]["2026-08-15T10:00:00Z"]
    assert set(at["wide"]) == {"3-4"}, at["wide"]
    assert set(at["trio"]) == {"2-3-4", "3-4-5"}, at["trio"]
    assert abs(at["wide"]["3-4"] - 4.0) < 1e-9, at["wide"]  # (3.1+4.9)/2
    assert abs(at["trio"]["3-4-5"] - 9999.9) < 1e-9, at["trio"]


def test_snapshot_report_keeps_win_rows_below_one():
    # 単勝は「出走馬の確定」に使うので、番兵以外（下限違反）は従来どおり残す。
    # ここを is_payout_odds で塞ぐと出走馬集合が縮んで ROI の分母が変わる。
    # #634 で win の番兵は「無い」が確定したので、99999.9 も**落ちない**（他券種の番兵値で
    # あって win の番兵ではない）。win 分岐が誤って is_payout_odds に変わると 0.5 が落ちて赤くなる。
    import snapshot_ev_report as SR

    rows = [
        dict(race_id="r1", date="2026-08-15", venue="新潟", race_num="1",
             bet_type="win", combination_key="1", odds="0.5", odds_high="", fetched_at="t1"),
        dict(race_id="r1", date="2026-08-15", venue="新潟", race_num="1",
             bet_type="win", combination_key="2", odds="99999.9", odds_high="", fetched_at="t1"),
        dict(race_id="r1", date="2026-08-15", venue="新潟", race_num="1",
             bet_type="win", combination_key="3", odds="4.2", odds_high="", fetched_at="t1"),
    ]
    got = SR.group_snapshots(rows)
    win = got["r1"]["times"]["t1"]["win"]
    assert set(win) == {1, 2, 3}, win  # win に番兵は無いので全行残る（0.5 の 1 番も従来どおり残る）


def test_fetch_wide_drops_both_end_sentinels():
    # 番兵を止めているのが `hi < lo` という**偶然**でないことを固定する（#621 の核心）。
    # netkeiba が両端に同じ番兵を返しても落ちること。
    import json

    import fetch_wide as FW

    payload = {"data": {"odds": {"5": {
        "0102": ["9999.9", "9999.9", "1"],   # 両端番兵 → 落ちる（hi < lo では捕まらない）
        "0103": ["9999.9", "0.0", "2"],      # 従来の形 → 落ちる
        "0203": ["3.1", "4.9", "3"],         # 正常 → 残る
    }}}}

    def fake_curl(_url):
        return json.dumps(payload).encode("utf-8")

    orig = FW.nk.curl
    FW.nk.curl = fake_curl
    try:
        got = FW.fetch_wide("dummy")
    finally:
        FW.nk.curl = orig
    assert set(got) == {(2, 3)}, got
    assert abs(got[(2, 3)] - 4.0) < 1e-9, got


def test_parse_wide_drops_sentinel_midpoint():
    # 二次防御。中点が番兵に一致するケース（両端とも同じ番兵だった行）だけ拾える。
    import tempfile

    import live_ev as L

    fd, path = tempfile.mkstemp(suffix=".tsv")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write("p1\t1-2\t9999.9\np1\t3-4\t21.6\n")
        got = L.parse_wide(path)
        assert got["p1"] == {(3, 4): 21.6}, got
    finally:
        os.unlink(path)


def _load_from(tmpdir, content, encoding="utf-8"):
    """正本ファイルを差し替えて _load_sentinels を呼ぶ（本物のファイルは触らない）。"""
    path = os.path.join(tmpdir, "netkeiba_sentinels.txt")
    if content is not None:
        with open(path, "w", encoding=encoding) as f:
            f.write(content)
    saved = G.SENTINELS_PATH
    try:
        G.SENTINELS_PATH = path
        return G._load_sentinels()
    finally:
        G.SENTINELS_PATH = saved


def test_broken_sentinel_file_fails_loudly_with_the_cause():
    # #635: import 時に読むため、壊れていれば解析スクリプト全部が起動できない。
    # そのとき「どのファイルの何行目が何だったか」が読めることを固定する。
    # **空タプルへのフォールバックはしない**——番兵が素通りして EV が静かに汚染されるため。
    import tempfile

    def own_text(e):
        """RuntimeError の**自作部分**だけを取り出す（原因例外の str() を混ぜない）。

        メッセージは `... {SENTINELS_PATH} ({e})。...` の形なので、素朴に
        `str(d) in str(e)` を見ると **原因例外の str() に載ったパス**で常に真になり、
        「自作文言からパスを落とす」変異が素通りする（1 巡目で実際に踏んだ）。

        取り出しに `split(" (")` を使わないのは、パスや文言に半角スペース + `(` が
        入ると自作文言を途中で切り、**本番コードが正しくてもテストが落ちる**ため
        （2 巡目で踏んだ。`TMPDIR` に括弧が入るだけで再現する）。原因例外の str() を
        差し引く形なら文言にも パスにも依存しない。
        """
        cause = str(e.__cause__) if e.__cause__ is not None else ""
        return str(e).replace(cause, "") if cause else str(e)

    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "netkeiba_sentinels.txt")

        # 1) ファイルが無い。**自作メッセージがパスと復旧手順を出すこと**を固定する。
        try:
            _load_from(d, None)
            raise AssertionError("欠落しても落ちなかった")
        except RuntimeError as e:
            assert path in own_text(e), own_text(e)
            assert "本番依存" in str(e), e
            assert isinstance(e.__cause__, OSError), e.__cause__

        # 2) 列数が 2 でない行（コメント行・旧 1 列書式の残骸など）— パス・行番号・中身が出る
        try:
            _load_from(d, "wide\t9999.9\n# ワイド\n")
            raise AssertionError("列数不正でも落ちなかった")
        except RuntimeError as e:
            assert f"{path}:2" in str(e) and "# ワイド" in str(e), e

        # 2b) 未知の券種ラベル — typo を「その行だけ静かに無効」にせず起動時に止める
        try:
            _load_from(d, "wide\t9999.9\ntansho\t1.0\n")
            raise AssertionError("未知ラベルでも落ちなかった")
        except RuntimeError as e:
            assert f"{path}:2" in str(e) and "'tansho'" in str(e), e

        # 2c) 値が数値でない — パス・行番号・中身と原因例外が出る
        try:
            _load_from(d, "wide\tabc\n")
            raise AssertionError("非数値でも落ちなかった")
        except RuntimeError as e:
            assert f"{path}:1" in str(e) and "'abc'" in str(e), e
            assert isinstance(e.__cause__, ValueError), e.__cause__

        # 3) 空ファイル（番兵ゼロ）— 正常として受理しない
        try:
            _load_from(d, "\n\n")
            raise AssertionError("空でも落ちなかった")
        except RuntimeError as e:
            assert "空" in str(e) and path in str(e), e

        # 4) UTF-8 以外で保存し直された（UnicodeDecodeError は ValueError のサブクラスで
        #    OSError ではないので、捕捉範囲を間違えると素の例外が漏れる）
        try:
            _load_from(d, "wide\t9999.9\n# ワイドの番兵\n", encoding="cp932")
            raise AssertionError("非 UTF-8 でも落ちなかった")
        except RuntimeError as e:
            assert path in own_text(e), own_text(e)
            assert "UTF-8" in str(e), e
            assert isinstance(e.__cause__, UnicodeDecodeError), e.__cause__

        # 5) 非有限値。番兵として登録しても比較が常に偽で**その値だけ黙って無効化**される。
        for bad, shown in (("nan", "'nan'"), ("inf", "'inf'"), ("1e400", "'1e400'")):
            try:
                _load_from(d, f"wide\t9999.9\ntrio\t{bad}\n")
                raise AssertionError(f"非有限値でも落ちなかった: {bad}")
            except RuntimeError as e:
                assert "非有限" in str(e) and f"{path}:2" in str(e), e
                assert shown in str(e), e

        # 6) 正常系: 空行・前後空白・末尾改行なし・同一券種の複数行は従来どおり読める
        assert _load_from(d, "  wide\t 9999.9  \n\nquinella\t99999.9") == {
            "wide": (9999.9,),
            "quinella": (99999.9,),
        }
        assert _load_from(d, "trio\t99999.9\ntrio\t88888.8\n") == {"trio": (99999.9, 88888.8)}


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")


if __name__ == "__main__":
    main()
