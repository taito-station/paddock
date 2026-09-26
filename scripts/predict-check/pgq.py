"""predict-check 共通の psql ヘルパ（#714）.

stdlib 制約（CI predict-check ジョブ）のため psql サブプロセスは維持し、次を一箇所で担保する:

- **接続パスワードを psql の argv・例外に出さない**: URL からパスワード（userinfo・クエリの
  `password=`）を抜き、libpq の環境変数 `PGPASSWORD` で渡す。`subprocess.CalledProcessError` は
  cmd（=URL 入り argv）を文字列化するので使わず、`PsqlError` には接続先（ホスト・ポート・DB 名）と
  psql の stderr だけを持たせる。パスワードを確実に抜けない URL（エンコードされていない `@` / `/`、
  環境変数で渡せない `sslpassword=`、key/value 形式）は値を載せずに拒否する（fail-closed）。
  ※ 環境変数は同一ユーザーなら `ps eww` で見える。また呼び出し側 CLI の `--db-url` に
  パスワード入り URL を渡すと Python 自身の argv に出るので、パスワードは `PADDOCK_DB_URL` で渡す。
- **行整列を壊さない**: `--csv` + `csv.reader`。TEXT にタブ・改行が入っても列がずれない
  （CR は subprocess の改行正規化で LF になる）。NULL と空文字はどちらも `''` になる
  （従来の `-At` + `COALESCE(...,'')` 前提と同じ）。
- **エラーで止まる**: SQL は stdin で渡し `ON_ERROR_STOP=1`（stdin 経由は既定だと SQL エラーでも
  exit 0 になる）。stdin 経由なので `-v name=value` の変数束縛（`:'name'`）が使える。
- **SQL はコード内の定数に限る**: stdin 経由の SQL は psql のメタコマンド（`\\!` 等）も解釈される。
  値は SQL 文字列に補間せず、必ず `variables` で束縛する（`:'name'` はクォート展開され再解釈されない）。
"""
import csv
import io
import os
import re
import subprocess
from urllib.parse import unquote

# 既定は 127.0.0.1（localhost が ::1 に先解決されると別の postgres に当たる既知事象・gen_predictions.py 参照）
DEFAULT_DB_URL = "postgres://paddock:paddock@127.0.0.1:5432/paddock"
# 利用者の変数名。psql 組み込み変数（ON_ERROR_STOP 等は大文字）を上書きさせないため小文字始まりに限る
_VAR_NAME = re.compile(r"[a-z_][a-z0-9_]*")
# JSON 列等の長いセルで csv の既定上限（131072 字）に当たらないようにする
csv.field_size_limit(2**31 - 1)


class PsqlError(RuntimeError):
    """psql の失敗。メッセージと属性に接続パスワードを含まない。"""

    def __init__(self, message, stderr=""):
        super().__init__(message)
        self.stderr = stderr


def db_url():
    """`PADDOCK_DB_URL`（未設定なら既定）を返す。"""
    return os.environ.get("PADDOCK_DB_URL", DEFAULT_DB_URL)


def split_secrets(url):
    """接続 URI を (パスワード抜きの URI, {環境変数名: 値}, 表示用の接続先) に分ける。

    libpq の URI 解釈に合わせる: userinfo は authority 先頭から最初の `@` か `/` まで
    （`?` では止まらない）。クエリはキー・値とも `%XX` だけをデコードする（`+` は空白にしない）。
    クエリの他のパラメータは原文のまま残す（再エンコードすると libpq の解釈が変わる）。
    パスワードを確実に抜けない URL は拒否する: userinfo 以外（クエリ含む）に `@` が残る（パスワード中の
    未エンコード `@` / `/` / `?` の疑い。正当な値の `@` は `%40` で書ける）・`sslpassword=`（渡す環境変数が
    libpq に無い）・URI 形式以外（key/value 形式）。
    空のパスワードは渡さない（libpq と同じく呼び出し側環境の `PGPASSWORD` に委ねる）。
    """
    scheme, sep, rest = url.partition("://")
    if not sep or scheme not in ("postgres", "postgresql"):
        # 値そのものは載せない（key/value 形式ならパスワードを含みうる）
        raise PsqlError("PADDOCK_DB_URL は postgres:// / postgresql:// 形式のみ対応")
    secrets = {}
    user_prefix = ""
    head = rest.split("/", 1)[0]
    if "@" in head:
        userinfo, _, after = rest.partition("@")
        user, colon, pw = userinfo.partition(":")
        if colon and pw:
            secrets["PGPASSWORD"] = unquote(pw)
        user_prefix = f"{user}@"
        rest = after
    # userinfo を外した残り（クエリも含む）に `@` があれば、パスワード中の未エンコード `/` `?` `@` の疑い
    # （`u:S3cr/t?x@h` は head に `@` が無く userinfo と判定されない / `u:a@b?c@h` は `b?c` が残る）。
    # クエリ値の正当な `@` は `%40` で書けるので一律に拒否してよい。
    if "@" in rest:
        raise PsqlError("PADDOCK_DB_URL の userinfo 以外（クエリ含む）に未エンコードの `@` があります"
                        "（パスワード・パラメータ中の記号は %40 / %2F / %3F 等にエンコードする）")
    main, qsep, query = rest.partition("?")
    kept = []
    if qsep:
        for part in query.split("&"):
            key, _, value = part.partition("=")
            key = unquote(key)
            if key == "sslpassword":
                raise PsqlError("PADDOCK_DB_URL の sslpassword= は非対応（psql に安全に渡す手段が無い）")
            if key == "password":
                if value:
                    secrets["PGPASSWORD"] = unquote(value)
            else:
                kept.append(part)
    safe = f"{scheme}://{user_prefix}{main}" + (f"?{'&'.join(kept)}" if kept else "")
    return safe, secrets, f"{scheme}://{main}"


def _run(sql, url, variables):
    safe_url, secrets, target = split_secrets(url if url is not None else db_url())
    cmd = ["psql", "-X", "-q", "-t", "--csv", "-v", "ON_ERROR_STOP=1"]
    for name, value in (variables or {}).items():
        if not _VAR_NAME.fullmatch(name):
            raise PsqlError(f"psql 変数名は小文字の識別子のみ: {name!r}")
        cmd += ["-v", f"{name}={value}"]
    cmd += ["-d", safe_url]
    env = dict(os.environ)
    env.setdefault("PGCONNECT_TIMEOUT", "5")
    env.update(secrets)
    try:
        proc = subprocess.run(cmd, input=sql, capture_output=True, text=True, env=env)
    except FileNotFoundError:
        raise PsqlError("psql が見つかりません（PATH を確認）") from None
    if proc.returncode != 0:
        stderr = proc.stderr.strip()
        raise PsqlError(f"psql 失敗 (exit {proc.returncode}, {target}): {stderr}", stderr=proc.stderr)
    return proc.stdout


def query_iter(sql, url=None, variables=None):
    """定数 SQL を実行して行（セル文字列の list）を 1 行ずつ返すイテレータ。失敗は `PsqlError`。

    大きな結果を 1 行ずつ捨てながら読むローダ向け（全行の list を抱えない）。psql の実行は最初の
    反復時に行われるので、`PsqlError` は反復する側（ローダ呼び出しを囲む try）で捕まえる。
    """
    out = _run(sql, url, variables)
    # splitlines を先にかけるとセル内改行で行が割れるので、ストリームのまま csv に渡す
    try:
        yield from csv.reader(io.StringIO(out))
    except csv.Error as e:
        raise PsqlError(f"psql の CSV 出力を解釈できません: {e}") from None


def query(sql, url=None, variables=None):
    """定数 SQL を実行して行（セル文字列の list）の list を返す。失敗は `PsqlError`。

    variables: `{name: value}` を `-v name=value` で束縛する（SQL 側は `:'name'` で安全にクォート展開）。
    `PGCONNECT_TIMEOUT` は呼び出し側の環境変数を尊重し、未設定時のみ 5 秒
    （DB が TCP は受けるが無応答のときの無言ハングを避ける）。
    """
    return list(query_iter(sql, url, variables))


def tsv_rows(text):
    """外部供給 TSV（`--rows-tsv` 等）を query() と同じ「行 = セル list」に揃える。空行は捨てる。"""
    return [line.split("\t") for line in text.splitlines() if line.strip()]
