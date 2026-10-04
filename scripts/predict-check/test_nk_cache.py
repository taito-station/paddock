#!/usr/bin/env python3
"""nk.ResultPages（result.html の HTML 単位キャッシュ・#714）のユニットテスト（自走式・stdlib のみ）。

ネットワークには出ない（fetch / sleep を注入）。固定する不変量:
(1) 1 レースの result.html 取得は 1 回（着順・確定オッズ・払戻の 3 パーサで共有・未完ページも同じ）
(2) 確定済みページだけを保存し、未完ページは保存しない（が HTML は返す）
(3) 切断・空のキャッシュは削除して取り直す。パーサ依存の検査は読み出しでは行わない
(4) 書き込みはアトミック（失敗時に最終パスへも tmp も残さない）
(5) 取得間隔は 3,334ms 以上で、一覧取得の後にも待つ。fetch_results.py / fetch_payouts.py は結果ページを
    nk.result_page 経由で取り（curl 直叩き・固定 sleep なし）、キャッシュに当たれば待たない（#763）
"""

import contextlib
import io
import os
import sys
import tempfile

import nk
import zure_sign_probe as zp

RID = "202607010210"

ROWS = (
    '<tr class="HorseList"><td class="Rank">1</td>'
    '<td class="Num Waku4"><div>4</div></td><td class="Num Txt_C"><div>4</div></td>'
    '<span class="HorseNameSpan">アルファ</span>'
    '<td class="Odds Txt_R"><span>8.6</span></td></tr>'
    '<tr class="HorseList"><td class="Rank">2</td>'
    '<td class="Num Waku5"><div>5</div></td><td class="Num Txt_C"><div>7</div></td>'
    '<span class="HorseNameSpan">ベータ</span>'
    '<td class="Odds Txt_R"><span>11.1</span></td></tr>'
)
PAYOUT = (
    '<table class="Payout_Detail_Table"><tr class="Tansho">'
    '<td class="Result"><div><span>4</span></div></td>'
    '<td class="Payout"><span>860円</span></td></tr></table>'
)


def _page(body):
    return f"<html><body>{body}</body>\n</html>\n".encode("utf-8")


COMPLETE = _page(ROWS + PAYOUT)
NO_PAYOUT = _page(ROWS)  # 着順は出たが払戻未掲載
NOT_READY = _page("<p>準備中</p>")


class FakeFetch:
    def __init__(self, raw):
        self.raw = raw
        self.urls = []

    def __call__(self, url):
        self.urls.append(url)
        return self.raw


def _pages(d, raw):
    fetch = FakeFetch(raw)
    sleeps = []
    return nk.ResultPages(cache_dir=d, fetch=fetch, sleep=sleeps.append), fetch, sleeps


def _read_all_three(pages):
    html = pages.html(RID)
    fin = {r["horse_num"]: r["rank"] for r in nk.parse_result(pages.html(RID), RID)}
    odds = zp.parse_result_odds(pages.html(RID))
    pay = nk.parse_payouts(pages.html(RID), RID).get("win", {})
    return html, fin, odds, pay


def test_complete_page_fetched_once_and_cached():
    with tempfile.TemporaryDirectory() as d:
        pages, fetch, sleeps = _pages(d, COMPLETE)
        _, fin, odds, pay = _read_all_three(pages)
        assert fin == {4: 1, 7: 2}
        assert odds == {4: 8.6, 7: 11.1}
        assert pay == {"4": 860}
        assert len(fetch.urls) == 1 and fetch.urls[0].endswith(f"race_id={RID}")
        assert sleeps == [nk.FETCH_PAUSE_SEC]  # ネットワーク取得 1 回ぶんだけ待つ
        assert pages._memo == {}  # 保存済みページはメモに抱えずディスクから読み直す
        with open(os.path.join(d, f"{RID}.html"), "rb") as f:
            assert f.read() == COMPLETE
        # 別プロセス相当（新インスタンス）はディスクから読み、取得も sleep もしない
        pages2, fetch2, sleeps2 = _pages(d, b"")
        assert pages2.html(RID) == COMPLETE.decode("utf-8")
        assert fetch2.urls == [] and sleeps2 == []
        assert [n for n in os.listdir(d) if n.endswith(".tmp")] == []


def test_incomplete_page_not_saved_but_returned_and_memoized():
    for raw in (NO_PAYOUT, NOT_READY):
        with tempfile.TemporaryDirectory() as d:
            pages, fetch, _ = _pages(d, raw)
            _, fin, _, pay = _read_all_three(pages)
            assert len(fetch.urls) == 1  # 未完ページも同一プロセス内では 1 回
            assert os.listdir(d) == []  # 保存しない（後日の再走で取り直す）
            assert pay == {}
            if raw is NO_PAYOUT:
                assert fin == {4: 1, 7: 2}  # 払戻未掲載でも着順は返る（現行 fetch_finish と同じ）


def test_truncated_cache_is_discarded_and_refetched():
    for broken in (b"", COMPLETE[: len(COMPLETE) // 2]):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, f"{RID}.html")
            with open(path, "wb") as f:
                f.write(broken)
            pages, fetch, _ = _pages(d, COMPLETE)
            assert pages.html(RID) == COMPLETE.decode("utf-8")
            assert len(fetch.urls) == 1
            with open(path, "rb") as f:
                assert f.read() == COMPLETE


def test_cached_page_not_revalidated_by_parser():
    # 完全な HTML だがパーサ的には未完（DOM 変化相当）でも、読み出しでは消さない
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, f"{RID}.html")
        with open(path, "wb") as f:
            f.write(NOT_READY)
        pages, fetch, _ = _pages(d, COMPLETE)
        assert pages.html(RID) == NOT_READY.decode("utf-8")
        assert fetch.urls == [] and os.path.exists(path)


def test_atomic_write_leaves_nothing_on_failure():
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "x.html")

        class Boom(Exception):
            pass

        orig = os.replace

        def failing_replace(src, dst):
            raise Boom()

        os.replace = failing_replace
        try:
            try:
                nk._atomic_write(path, COMPLETE)
                raise AssertionError("Boom が出るはず")
            except Boom:
                pass
        finally:
            os.replace = orig
        assert os.listdir(d) == []


def test_default_fetch_resolves_nk_curl_at_call_time():
    # 既定 fetch は呼び出し時に nk.curl を引く（monkeypatch が効く）
    with tempfile.TemporaryDirectory() as d:
        calls = []
        orig = nk.curl
        nk.curl = lambda url: calls.append(url) or NOT_READY
        try:
            nk.ResultPages(cache_dir=d, sleep=lambda s: None).html(RID)
        finally:
            nk.curl = orig
        assert len(calls) == 1


def test_truncated_fetch_is_returned_but_not_saved():
    # パーサ的には完全（着順＋単勝払戻あり）でも `</html>` が欠けた取得結果は保存しない
    cut = (ROWS + PAYOUT).encode("utf-8")
    with tempfile.TemporaryDirectory() as d:
        pages, fetch, _ = _pages(d, cut)
        assert nk.parse_payouts(pages.html(RID), RID).get("win") == {"4": 860}
        assert pages.html(RID) and len(fetch.urls) == 1
        assert os.listdir(d) == []


def test_unsaved_page_warns_with_reason():
    with tempfile.TemporaryDirectory() as d:
        pages, _, _ = _pages(d, NO_PAYOUT)
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            pages.html(RID)
        assert "保存しません" in err.getvalue() and "単勝払戻なし" in err.getvalue(), err.getvalue()


def test_fetch_failure_still_paces():
    with tempfile.TemporaryDirectory() as d:
        sleeps = []

        def failing(url):
            raise RuntimeError("curl 失敗 (exit 22)")

        pages = nk.ResultPages(cache_dir=d, fetch=failing, sleep=sleeps.append)
        try:
            pages.html(RID)
            raise AssertionError("RuntimeError が出るはず")
        except RuntimeError:
            pass
        assert sleeps == [nk.FETCH_PAUSE_SEC]


def test_invalid_race_id_is_rejected():
    with tempfile.TemporaryDirectory() as d:
        pages, fetch, _ = _pages(d, COMPLETE)
        for bad in ("../../etc/x", "2026070102100", "20260701021", "２０２６０７０１０２１０"):
            try:
                pages.html(bad)
                raise AssertionError(f"ValueError が出るはず: {bad!r}")
            except ValueError:
                pass
        assert fetch.urls == []


def test_is_truncated():
    assert nk.is_truncated(b"")
    assert nk.is_truncated(b"<html><body>")
    assert not nk.is_truncated(COMPLETE)
    assert not nk.is_truncated(b"<HTML></HTML>\n")


LIST_HTML = f'<a href="../race/shutuba.html?race_id={RID}">1R</a>'.encode("utf-8")
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))


def test_fetch_pause_meets_bulk_floor():
    # バルク取得は 0.3 rps 以下＝3,334ms 以上（knowledge/netkeiba-datasource.md 決定ログ #721・#763）
    assert nk.FETCH_PAUSE_SEC * 1000 >= 3334, nk.FETCH_PAUSE_SEC


@contextlib.contextmanager
def _patched_network(list_raw, page_raw, cache_dir):
    """nk.curl（一覧ページ）・time.sleep・既定の ResultPages を差し替える。ネットワークに出ない。"""
    curl_urls, sleeps = [], []
    page_fetch = FakeFetch(page_raw)
    saved = (nk.curl, nk.time.sleep, nk._default_pages)

    def fake_curl(url, timeout=25):
        curl_urls.append(url)
        return list_raw

    nk.curl = fake_curl
    # stdlib の time.sleep ごと差し替える（スクリプト側に固定の time.sleep が戻っても sleeps に出て落ちるように）
    nk.time.sleep = sleeps.append
    nk._default_pages = nk.ResultPages(cache_dir=cache_dir, fetch=page_fetch, sleep=sleeps.append)
    try:
        yield curl_urls, page_fetch, sleeps
    finally:
        nk.curl, nk.time.sleep, nk._default_pages = saved


def test_list_race_ids_pauses_after_fetch():
    with tempfile.TemporaryDirectory() as d, _patched_network(LIST_HTML, b"", d) as (curl_urls, _, sleeps):
        assert nk.list_race_ids("20260701") == [RID]
        assert len(curl_urls) == 1
        assert sleeps == [nk.FETCH_PAUSE_SEC]  # 一覧→1 レース目の取得が連続しない


def _run_script(name, argv):
    import runpy

    out, err = io.StringIO(), io.StringIO()
    saved_argv = sys.argv
    sys.argv = [name] + argv
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            runpy.run_path(os.path.join(SCRIPT_DIR, name), run_name="__main__")
    finally:
        sys.argv = saved_argv
    return out.getvalue()


def _check_script_uses_result_page(name, check_out):
    with tempfile.TemporaryDirectory() as d:
        with _patched_network(LIST_HTML, COMPLETE, d) as (curl_urls, page_fetch, sleeps):
            check_out(_run_script(name, ["20260701"]))
            assert len(curl_urls) == 1 and "race_list_sub" in curl_urls[0]  # 結果ページは curl を直接叩かない
            assert len(page_fetch.urls) == 1  # 結果ページは nk.result_page 経由で 1 回
            assert sleeps == [nk.FETCH_PAUSE_SEC] * 2  # 一覧 1 回 + 結果ページ 1 回。固定 sleep は無い
        # 2 回目（別プロセス相当）は確定済みキャッシュに当たり、結果ページの取得も待ちもしない
        with _patched_network(LIST_HTML, b"", d) as (_, page_fetch, sleeps):
            check_out(_run_script(name, ["20260701"]))
            assert page_fetch.urls == []
            assert sleeps == [nk.FETCH_PAUSE_SEC]  # 一覧の 1 回だけ


def test_fetch_payouts_script_uses_result_page():
    import json

    def check(out):
        assert json.loads(out)[0]["payouts"].get("win") == {"4": 860}

    _check_script_uses_result_page("fetch_payouts.py", check)


def test_fetch_results_script_uses_result_page():
    import json

    def check(out):
        assert {r["horse_num"]: r["rank"] for r in json.loads(out)[0]["rows"]} == {4: 1, 7: 2}

    _check_script_uses_result_page("fetch_results.py", check)


def main() -> int:
    fails = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            try:
                fn()
                print(f"ok   {name}")
            except AssertionError as e:
                fails += 1
                print(f"FAIL {name}: {e}")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
