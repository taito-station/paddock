#!/usr/bin/env python3
"""pgq.py のユニットテスト（自走式・stdlib のみ・CI predict-check 対象）。

実 DB は使わない。PATH 先頭に偽 psql（argv・関係する環境変数を JSON で書き出し、
指定の stdout / exit を返すスクリプト）を置き、次を固定する:
(1) argv・例外にパスワードが出ない（PGPASSWORD で渡る）
(2) CSV のセル内タブ・改行が 1 セルのまま返る
(3) URL 分解の細則（percent-encoding・クエリ password・IPv6・パスワード無し）
"""

import json
import os
import stat
import sys
import tempfile

import pgq

SECRET = "s3cr@t/pw"  # `@` と `/` を含む（URL では %40 / %2F にエンコードされる）
URL = "postgres://paddock:s3cr%40t%2Fpw@127.0.0.1:5432/paddock"

FAKE_PSQL = """#!/usr/bin/env python3
import json, os, sys
with open(os.environ["FAKE_PSQL_LOG"], "w") as f:
    json.dump({"argv": sys.argv[1:], "stdin": sys.stdin.read(),
               "PGPASSWORD": os.environ.get("PGPASSWORD"),
               "PGCONNECT_TIMEOUT": os.environ.get("PGCONNECT_TIMEOUT")}, f)
with open(os.environ["FAKE_PSQL_STDOUT_FILE"], encoding="utf-8") as f:
    sys.stdout.write(f.read())
sys.stderr.write(os.environ.get("FAKE_PSQL_STDERR", ""))
sys.exit(int(os.environ.get("FAKE_PSQL_EXIT", "0")))
"""


def _with_fake_psql(fn, stdout="", stderr="", exit_code=0, env_extra=None):
    """偽 psql を PATH 先頭に置いて fn() を呼び、(戻り値 or 例外, 偽 psql が見た内容) を返す。"""
    saved = dict(os.environ)
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "psql")
        with open(path, "w") as f:
            f.write(FAKE_PSQL)
        os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)
        log = os.path.join(d, "log.json")
        # stdout はファイルで渡す（Linux は環境変数 1 個あたり 128KiB 上限で、長大セルのテストが起動できない）
        out_file = os.path.join(d, "stdout.txt")
        with open(out_file, "w", encoding="utf-8") as f:
            f.write(stdout)
        os.environ.update({"PATH": d + os.pathsep + saved.get("PATH", ""), "FAKE_PSQL_LOG": log,
                           "FAKE_PSQL_STDOUT_FILE": out_file, "FAKE_PSQL_STDERR": stderr,
                           "FAKE_PSQL_EXIT": str(exit_code)})
        os.environ.pop("PGPASSWORD", None)
        os.environ.pop("PGCONNECT_TIMEOUT", None)
        os.environ.update(env_extra or {})
        try:
            try:
                result = fn()
            except pgq.PsqlError as e:
                result = e
            seen = None  # psql を起動する前に弾かれたら log は無い
            if os.path.exists(log):
                with open(log) as f:
                    seen = json.load(f)
        finally:
            os.environ.clear()
            os.environ.update(saved)
    return result, seen


def test_password_goes_to_env_not_argv():
    rows, seen = _with_fake_psql(lambda: pgq.query("SELECT 1", url=URL), stdout="1\n")
    assert rows == [["1"]]
    assert seen["PGPASSWORD"] == SECRET
    argv = " ".join(seen["argv"])
    assert "s3cr" not in argv, argv
    assert "postgres://paddock@127.0.0.1:5432/paddock" in seen["argv"]
    # SQL は stdin・ON_ERROR_STOP 付き・CSV・ヘッダ無し
    assert seen["stdin"] == "SELECT 1"
    for flag in ("--csv", "-t", "ON_ERROR_STOP=1", "-X"):
        assert flag in seen["argv"], flag
    assert seen["PGCONNECT_TIMEOUT"] == "5"


def test_failure_message_has_no_password():
    err, _ = _with_fake_psql(lambda: pgq.query("SELECT 1", url=URL),
                             stderr="psql: error: connection refused\n", exit_code=2)
    assert isinstance(err, pgq.PsqlError), err
    msg = str(err)
    assert "s3cr" not in msg and "%40" not in msg, msg
    assert "connection refused" in msg and "exit 2" in msg
    assert "connection refused" in err.stderr


def test_psql_missing_is_psql_error():
    saved = dict(os.environ)
    with tempfile.TemporaryDirectory() as empty:
        os.environ["PATH"] = empty  # psql の無い PATH
        try:
            pgq.query("SELECT 1", url=URL)
            raise AssertionError("PsqlError が出るはず")
        except pgq.PsqlError as e:
            assert "psql が見つかりません" in str(e)
        finally:
            os.environ.clear()
            os.environ.update(saved)


def test_csv_cells_keep_tabs_and_newlines():
    out = 'a,"x\ty\nz",,"{""k"":""v, w""}"\nb,plain,t,\n'
    rows, _ = _with_fake_psql(lambda: pgq.query("SELECT", url=URL), stdout=out)
    assert rows == [["a", "x\ty\nz", "", '{"k":"v, w"}'], ["b", "plain", "t", ""]], rows


def test_empty_result_is_empty_list():
    rows, _ = _with_fake_psql(lambda: pgq.query("SELECT", url=URL), stdout="")
    assert rows == []


def test_variables_are_bound_with_v():
    _, seen = _with_fake_psql(lambda: pgq.query("SELECT :'date'", url=URL,
                                                variables={"date": "2026-09-26"}))
    i = seen["argv"].index("date=2026-09-26")
    assert seen["argv"][i - 1] == "-v"


def test_connect_timeout_respects_caller_env():
    _, seen = _with_fake_psql(lambda: pgq.query("SELECT 1", url=URL),
                              env_extra={"PGCONNECT_TIMEOUT": "30"})
    assert seen["PGCONNECT_TIMEOUT"] == "30"


def test_no_password_keeps_existing_pgpassword():
    _, seen = _with_fake_psql(lambda: pgq.query("SELECT 1", url="postgresql://u@h/db"),
                              env_extra={"PGPASSWORD": "from-env"})
    assert seen["PGPASSWORD"] == "from-env"


def test_split_secrets_variants():
    pw = lambda v: {"PGPASSWORD": v}  # noqa: E731
    assert pgq.split_secrets(URL) == (
        "postgres://paddock@127.0.0.1:5432/paddock", pw(SECRET), "postgres://127.0.0.1:5432/paddock")
    assert pgq.split_secrets("postgresql://u@h:5432/db")[:2] == ("postgresql://u@h:5432/db", {})
    assert pgq.split_secrets("postgres://h/db")[:2] == ("postgres://h/db", {})
    assert pgq.split_secrets("postgres://u:p@[::1]:5432/db")[:2] == ("postgres://u@[::1]:5432/db", pw("p"))
    # libpq と同じく userinfo は最初の `@` か `/` まで（`?` では止まらない）: 未エンコードの `?` を含むパスワード
    assert pgq.split_secrets("postgres://u:p?x@h/db")[:2] == ("postgres://u@h/db", pw("p?x"))
    assert pgq.split_secrets("postgres://u:p?x@h")[:2] == ("postgres://u@h", pw("p?x"))
    # クエリの password= も抜き、他のパラメータは原文のまま残す（%20 を + に再エンコードしない）
    assert pgq.split_secrets("postgres://u@h/db?sslmode=disable&password=qq")[:2] == (
        "postgres://u@h/db?sslmode=disable", pw("qq"))
    assert pgq.split_secrets("postgres://u@h/db?password=qq")[:2] == ("postgres://u@h/db", pw("qq"))
    # 値の + は空白にしない（libpq と同じ）
    assert pgq.split_secrets("postgres://u@h/db?options=-c%20x%3Dy&password=p%2Bq+r")[:2] == (
        "postgres://u@h/db?options=-c%20x%3Dy", pw("p+q+r"))
    # キーも %XX デコードして比較する（libpq はキーもデコードする）
    assert pgq.split_secrets("postgres://u@h/db?pass%77ord=zz")[:2] == ("postgres://u@h/db", pw("zz"))
    # 空のパスワードは渡さない（呼び出し側環境の PGPASSWORD に委ねる）
    assert pgq.split_secrets("postgres://u:@h/db")[:2] == ("postgres://u@h/db", {})
    assert pgq.split_secrets("postgres://u@h/db?password=")[:2] == ("postgres://u@h/db", {})


def test_unsplittable_urls_are_rejected_without_echo():
    # パスワードを確実に抜けない URL は値を載せずに拒否（fail-closed）
    for bad in ("postgres://u:ab/secret1@h/db",         # 未エンコードの `/`
                "postgres://u:ab@secret1@h/db",         # 未エンコードの `@`
                "postgres://u:secret1/t?x@h:5999/db",   # `/` の後に `?`（head に `@` が無い）
                "postgres://u:a@secret1?c@h/db",        # `@` の後に `?`（残りがクエリ側に回る）
                "postgres://u:secret1?cd/ef@h/db",      # `?` の後に `/`
                "postgres://u:secret1/y?z=1@h/db",      # `/` の後にクエリ風の `?z=1`
                "postgres://u@h/db?sslpassword=secret1",  # 環境変数で渡せない
                "postgres://u@h/db?ssl%70assword=secret1"):
        try:
            pgq.split_secrets(bad)
            raise AssertionError(f"PsqlError が出るはず: {bad}")
        except pgq.PsqlError as e:
            assert "secret1" not in str(e), str(e)


def test_failure_message_shows_only_target():
    err, _ = _with_fake_psql(lambda: pgq.query("SELECT 1", url="postgres://usr:pw@h:5432/db"),
                             stderr="boom\n", exit_code=2)
    assert isinstance(err, pgq.PsqlError), err
    assert "postgres://h:5432/db" in str(err) and "usr" not in str(err), str(err)


def test_empty_password_keeps_existing_pgpassword():
    _, seen = _with_fake_psql(lambda: pgq.query("SELECT 1", url="postgres://u:@h/db"),
                              env_extra={"PGPASSWORD": "from-env"})
    assert seen["PGPASSWORD"] == "from-env"


def test_query_iter_yields_rows_lazily():
    it = pgq.query_iter("SELECT", url=URL)  # 反復するまで psql は起動しない
    rows, seen = _with_fake_psql(lambda: list(it), stdout="a,1\nb,2\n")
    assert rows == [["a", "1"], ["b", "2"]] and seen is not None


def test_long_cell_over_default_csv_limit():
    big = "x" * 200_000  # csv の既定上限 131072 字を超える
    rows, _ = _with_fake_psql(lambda: pgq.query("SELECT", url=URL), stdout=f"a,{big}\n")
    assert rows == [["a", big]], type(rows)


def test_variable_name_is_validated():
    # 偽 psql は成功（exit 0）する。PsqlError は名前検証からしか出ず、psql も起動しない
    for bad in ("ON_ERROR_STOP", "a=b", "x y", ""):
        err, seen = _with_fake_psql(lambda: pgq.query("SELECT 1", url=URL, variables={bad: "0"}))
        assert isinstance(err, pgq.PsqlError), (bad, err)
        assert seen is None, bad


def test_csv_error_is_psql_error():
    def broken_reader(_stream):
        raise pgq.csv.Error("field larger than field limit")

    orig = pgq.csv.reader
    pgq.csv.reader = broken_reader
    try:
        err, _ = _with_fake_psql(lambda: pgq.query("SELECT", url=URL), stdout="a,b\n")
    finally:
        pgq.csv.reader = orig
    assert isinstance(err, pgq.PsqlError), err


def test_non_uri_is_rejected_without_echo():
    try:
        pgq.split_secrets("host=h password=topsecret")
        raise AssertionError("PsqlError が出るはず")
    except pgq.PsqlError as e:
        assert "topsecret" not in str(e)


def test_tsv_rows():
    assert pgq.tsv_rows("a\tb\n\n c\td\n") == [["a", "b"], [" c", "d"]]


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
