#!/usr/bin/env python3
"""live_ev.py --emit-json の JSON を Postgres `live_ev_snapshots` へ upsert する（#260・ADR 0064）.

live_ev.py 本体は DB 非依存を保つ設計のため、永続化はこの薄いスクリプトが担う（refresh_ev.sh が呼ぶ）。
外部依存を増やさない方針（predict-check は curl + 標準ライブラリのみ）に合わせ、psycopg 等は使わず
`psql` に upsert SQL を流す。emit-json は pid ローカル値のみ持つため、`date` は引数、`post_time` は
`race_cards` からのサブクエリで補完する（race_id は emit-json の race_id＝paddock race_id をそのまま使う）。

冪等性: `captured_at` は 1 サイクル 1 値をサイクル境界時刻として与える（refresh_ev.sh / スケジューラ）。
`(race_id, captured_at)` の一意制約 + ON CONFLICT DO UPDATE で、同一サイクルの再実行を冪等にする。
"""
import argparse
import json
import math
import os
import re
import subprocess
import sys

_DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
# captured_at は UTC rfc3339（`...Z` 終端）に限定する。read API の window `ORDER BY captured_at
# DESC` と interactor の last_updated=max() は辞書順=時刻順を前提とし、これは全 captured_at が
# 同一表記（UTC Z）のときだけ成立する。`+09:00` 等の混在で「最新サイクル」判定が静かに壊れるため、
# 正本フロー（refresh_ev.sh の `date -u ...Z`）と同じ Z 終端だけを許す。
_TS_RE = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$")


def lit_str(s):
    """SQL 文字列リテラル（`standard_conforming_strings=on` 前提で単一引用符のみ 2 重化）。
    前提は build_sql が `SET LOCAL standard_conforming_strings TO on` で明示保証する。"""
    return "'" + str(s).replace("'", "''") + "'"


def lit_num(x):
    if x is None:
        return "NULL"
    f = float(x)
    # NaN/Inf は数値リテラルにできず psql が構文エラーにするため NULL に落とす（防御）。
    return repr(f) if math.isfinite(f) else "NULL"


def lit_int(x):
    return "NULL" if x is None else str(int(x))


def lit_bool(b):
    return "TRUE" if b else "FALSE"


def lit_jsonb(obj):
    return lit_str(json.dumps(obj, ensure_ascii=False)) + "::jsonb"


def build_sql(payload, date, captured_at):
    """emit-json payload → 1 トランザクションの upsert SQL 文字列。"""
    # lit_str のエスケープは standard_conforming_strings=on 前提。接続設定に依存せず前提を
    # 満たすため、トランザクション先頭で明示 SET する（多層防御。既定 on だが強制する）。
    stmts = ["BEGIN;", "SET LOCAL standard_conforming_strings TO on;"]
    for r in payload.get("races", []):
        race_id = r["race_id"]
        cols = ("date, race_id, venue, race_no, post_time, captured_at, verdict, roi, "
                "konsen, axis, axis_prob, axis_win_odds, odds_missing, slip, raw")
        values = ", ".join([
            lit_str(date),
            lit_str(race_id),
            lit_str(r["venue"]),
            lit_int(r["race_no"]),
            # post_time は race_cards から補完（emit-json は持たない）。無ければ NULL。
            f"(SELECT post_time FROM race_cards WHERE race_id = {lit_str(race_id)})",
            lit_str(captured_at),
            lit_str(r["verdict"]),
            lit_num(r["roi"]),
            lit_bool(r["konsen"]),
            lit_int(r["axis"]),
            lit_num(r["axis_prob"]),
            lit_num(r.get("axis_win_odds")),
            lit_bool(r["odds_missing"]),
            lit_jsonb(r["slip"]),
            lit_jsonb(r),  # raw = races[] 要素 1 件（原本）
        ])
        stmts.append(
            f"INSERT INTO live_ev_snapshots ({cols}) VALUES ({values})\n"
            "ON CONFLICT (race_id, captured_at) DO UPDATE SET\n"
            "  date = excluded.date, venue = excluded.venue, race_no = excluded.race_no,\n"
            "  post_time = excluded.post_time, verdict = excluded.verdict, roi = excluded.roi,\n"
            "  konsen = excluded.konsen, axis = excluded.axis, axis_prob = excluded.axis_prob,\n"
            "  axis_win_odds = excluded.axis_win_odds, odds_missing = excluded.odds_missing,\n"
            "  slip = excluded.slip, raw = excluded.raw;"
        )
    stmts.append("COMMIT;")
    return "\n".join(stmts)


def main():
    ap = argparse.ArgumentParser(description="live_ev emit-json → live_ev_snapshots upsert")
    ap.add_argument("--json", required=True, help="live_ev.py --emit-json の出力 JSON パス")
    ap.add_argument("--date", required=True, help="開催日 YYYY-MM-DD")
    ap.add_argument("--captured-at", required=True,
                    help="サイクル境界時刻（UTC rfc3339, 例 2026-06-20T15:20:00Z）")
    ap.add_argument("--db-url", default=os.environ.get("PADDOCK_DB_URL"),
                    help="Postgres 接続 URL（既定: 環境変数 PADDOCK_DB_URL）")
    args = ap.parse_args()
    if not args.db_url:
        sys.exit("PADDOCK_DB_URL または --db-url が必要です")
    # 形式検証（不正形式が黙って DB に入り read API の WHERE date=$1 で無言 0 ヒットになるのを防ぐ）。
    if not _DATE_RE.match(args.date):
        sys.exit(f"--date は YYYY-MM-DD 形式: {args.date}")
    if not _TS_RE.match(args.captured_at):
        sys.exit(f"--captured-at は rfc3339（例 2026-06-20T15:20:00Z）: {args.captured_at}")

    with open(args.json, encoding="utf-8") as f:
        payload = json.load(f)
    races = payload.get("races", [])
    if not races:
        print("[persist] 対象レース 0 件（スキップ）", file=sys.stderr)
        return
    sql = build_sql(payload, args.date, args.captured_at)
    subprocess.run(
        ["psql", args.db_url, "-q", "-v", "ON_ERROR_STOP=1"],
        input=sql, text=True, check=True,
    )
    print(f"[persist] {len(races)} レースを live_ev_snapshots へ upsert（captured_at={args.captured_at}）",
          file=sys.stderr)


if __name__ == "__main__":
    main()
