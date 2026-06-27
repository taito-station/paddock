#!/usr/bin/env python3
"""発走時刻ベースで「これから発走する」レースを DB から絞り込む（締切前 prefetch 用, #237）.

`upcoming_races.py`（#197）は netkeiba の race_list を都度スクレイプして発走時刻を得るが、
#235 で `race_cards.post_time`（HH:MM）が DB に入ったため、本スクリプトは **DB の post_time
だけ** で同じ窓判定を行う（追加スクレイプ無し）。窓判定の純粋関数 `select_upcoming` は
`upcoming_races.py` から再利用する。

前提: その日の出馬表（post_time 入り）は通常の朝の `paddock-fetch-card` 運用で既に DB に
投入済みであること。未投入の日は対象 0 件で何も出力しない（呼び出し側は no-op）。

使い方:
    python3 upcoming_races_db.py YYYY-MM-DD [--window-min N] [--at HH:MM]
例:
    python3 upcoming_races_db.py 2026-06-20 --window-min 30      # 発走 30 分以内の未発走
    python3 upcoming_races_db.py 2026-06-20 --at 15:10           # 現在時刻を 15:10 とみなす（検証用）

出力: 対象 paddock race_id を発走時刻順に 1 行ずつ（fetch-card 投入用）。
DB 接続は環境変数 PADDOCK_DB_URL（既定 postgres://paddock:paddock@127.0.0.1:5432/paddock）。
host は localhost ではなく 127.0.0.1 を使う（#212, ::1 先解決で別 postgres に当たる事故回避）。
"""
import argparse
import os
import re
import subprocess
import sys
from datetime import datetime
from zoneinfo import ZoneInfo

from upcoming_races import select_upcoming, to_minutes, valid_hhmm

JST = ZoneInfo("Asia/Tokyo")
DEFAULT_DB_URL = "postgres://paddock:paddock@127.0.0.1:5432/paddock"


def valid_date(s: str) -> str:
    """argparse 用: 開催日を `YYYY-MM-DD`（race_cards.date と同形式）に検証する。

    形式（ゼロ埋め YYYY-MM-DD）と暦妥当性の両方を見る。形式だけだと `2026-13-45` のような
    非実在日も通り、SQL は無害（注入不能）だが「形式 OK だが存在しない日→常に 0 件」を
    静かに招くため、`strptime` で暦も検証する。
    """
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", s):
        raise argparse.ArgumentTypeError(f"YYYY-MM-DD 形式で指定してください: {s!r}")
    try:
        datetime.strptime(s, "%Y-%m-%d")
    except ValueError:
        raise argparse.ArgumentTypeError(f"存在しない日付です: {s!r}")
    return s


def select_from_rows(rows, now_min: int, window_min: int):
    """`(race_id, post_time)` 行から「now 以降かつ now+window 以内」の race_id を返す純粋関数.

    post_time が None/空の行は除外する（カード投入済みだが発走時刻未取得＝判定不能）。
    窓判定そのものは `upcoming_races.select_upcoming` に委譲し、DB 経路と netkeiba 経路で
    同一ロジックを保つ。ネットワーク・時刻取得・DB に依存しないためテスト可能。
    """
    post_times = {rid: pt for rid, pt in rows if pt}
    return select_upcoming(post_times, now_min, window_min)


def fetch_rows(date: str, db_url: str):
    """race_cards から `(race_id, post_time)` を取得する。post_time NULL 行は SQL 側で除外。"""
    sql = (
        "SELECT race_id, post_time FROM race_cards "
        f"WHERE date = '{date}' AND post_time IS NOT NULL"
    )
    # date は valid_date で厳格検証済みなので文字列展開でも注入されない。
    # 接続タイムアウトを付ける（無人 launchd ジョブで DB が TCP は受けるが無応答＝ハング時に、
    # 選択段階で無言ハングして 5 分毎に別プロセスが積み上がるのを防ぐ。fast-fail だけでなく
    # ハングも救う）。PGCONNECT_TIMEOUT は URL を弄らず psql に効かせられる。
    env = {**os.environ, "PGCONNECT_TIMEOUT": os.environ.get("PGCONNECT_TIMEOUT", "5")}
    proc = subprocess.run(
        ["psql", db_url, "-t", "-A", "-F", "\t", "-c", sql],
        capture_output=True, text=True, env=env,
    )
    if proc.returncode != 0:
        # 無人運用（launchd/cron）での停止時に根因（connection refused 等）が消えないよう、
        # psql の stderr を呼び出し側の stderr（launchd.err.log）へ転記してから非0終了する。
        sys.stderr.write(proc.stderr)
        raise SystemExit(f"psql 失敗 (exit {proc.returncode})")
    out = proc.stdout
    rows = []
    for line in out.splitlines():
        if not line.strip():
            continue
        rid, _, pt = line.partition("\t")
        rows.append((rid, pt))
    return rows


def main(argv=None):
    ap = argparse.ArgumentParser(description="DB post_time で締切前レースを絞る (#237)")
    ap.add_argument("date", type=valid_date, help="開催日 YYYY-MM-DD")
    ap.add_argument("--window-min", type=int, default=30,
                    help="発走まで何分以内を対象にするか（既定 30）")
    ap.add_argument("--at", metavar="HH:MM", type=valid_hhmm,
                    help="現在時刻の上書き（テスト/検証用。既定はシステム JST 時刻）")
    args = ap.parse_args(argv)

    db_url = os.environ.get("PADDOCK_DB_URL", DEFAULT_DB_URL)
    rows = fetch_rows(args.date, db_url)

    if args.at:
        now_min = to_minutes(args.at)
    else:
        now = datetime.now(JST)
        now_min = now.hour * 60 + now.minute

    ids = select_from_rows(rows, now_min, args.window_min)
    for rid in ids:
        print(rid)
    print(f"# {len(ids)} races (of {len(rows)} carded with post_time)", file=sys.stderr)


if __name__ == "__main__":
    main()
