"""snapshot 取りこぼし検知（#264）.

`race_cards.post_time`（発走時刻 HH:MM JST, #235）と `race_odds_snapshots`（締切前オッズの
時系列, #232）を突き合わせ、開催日×レース単位で「最終 snapshot が発走の何分前で止まっているか」
を出す。締切前 prefetch（#237）が Mac スリープ/不在で取りこぼした（= 最終 snapshot が発走から
大きく前／そもそも朝の 1 本だけ）レースを可視化する。snapshot は過去オッズ再取得不能で復元
できないため、欠落を後から気付けるようにするのが目的（#248 の年間 +EV 集計の信頼性前提）。

判定:
  - lag_min = post(分) − 最終 snapshot(分, JST)。「最終 snapshot が発走の何分前か」。
  - ok  : lag_min <= max_lag（締切直前まで取れている）。post 以降の取得（lag<0）も ok。
  - gap : lag_min > max_lag（最終 snapshot が発走から離れて止まっている＝取りこぼし疑い）。
  - none: snapshot が 1 本も無い。

使い方:
    python3 snapshot_coverage.py --date 2026-06-27 [--max-lag-min 10]
    # テスト/再現用に外部 TSV を注入（DB に触らない）
    python3 snapshot_coverage.py --rows-tsv rows.tsv --max-lag-min 10

--rows-tsv 形式（タブ区切り）: race_id  venue  race_num  post_time(HH:MM)  last_fetched_at(rfc3339 or 空)  n_snaps
"""
import argparse
import os
import re
import subprocess
import sys

DEFAULT_DB_URL = "postgres://paddock:paddock@127.0.0.1:5432/paddock"
JST_OFFSET_MIN = 9 * 60  # fetched_at は UTC rfc3339。JST 日中レースは UTC でも同日 00:00-07:59。


def hhmm_to_min(hhmm: str):
    """'HH:MM' を 0-1439 の分へ。不正は None。"""
    m = re.fullmatch(r"([0-9]{1,2}):([0-9]{2})", hhmm.strip())
    if not m:
        return None
    h, mi = int(m.group(1)), int(m.group(2))
    if h > 23 or mi > 59:
        return None
    return h * 60 + mi


def fetched_at_to_jst_min(fetched_at: str):
    """rfc3339 UTC の fetched_at（例 2026-06-27T05:40:45.19+00:00）を JST の分(0-1439)へ。

    save_race_odds が +00:00 固定で書く前提（migration 20260625）で時刻部の HH:MM だけ見る。
    JST 日中レース（09:00-16:59）の snapshot は UTC 00:00-07:59 に収まり日跨ぎしないので、
    (UTC 分 + 540) % 1440 で JST 分になる。パースできなければ None。

    区切りは `T`（rfc3339 TEXT）とスペース（timestamptz/datestyle 変化で psql が `2026-06-27
    05:40:45+00` と出すケース）の両方を許容し、フォーマット差で全件 none に化ける事故を防ぐ。

    この HH:MM が **UTC である前提**で +540 固定シフトする。将来 timestamptz 化やセッション
    TimeZone 変化で UTC 以外のオフセット付き文字列が来た場合、黙って二重シフト（gap→ok の沈黙
    故障）させないため、UTC 以外のオフセットを検出したら変換を諦め None を返す（→ bad_ts で顕在化）。
    """
    m = re.search(r"[T ]([0-9]{2}):([0-9]{2})", fetched_at)
    if not m:
        return None
    # 末尾のタイムゾーン表記を検査。UTC（Z / +00 / +0000 / +00:00）以外は拒否（二重シフト防止）。
    tz = re.search(r"(Z|[+-][0-9]{2}:?[0-9]{2}|[+-][0-9]{2})$", fetched_at.strip())
    if tz and tz.group(0) not in ("Z", "+00", "+0000", "+00:00", "-00", "-0000", "-00:00"):
        return None
    utc_min = int(m.group(1)) * 60 + int(m.group(2))
    return (utc_min + JST_OFFSET_MIN) % 1440


def classify(post_min, last_snap_min, n_snaps, max_lag_min):
    """1 レースの取りこぼし判定（純関数）。返り値 dict: status/lag_min。

    status: 'none'（snapshot 無し）/ 'bad_ts'（snapshot はあるが fetched_at をパース不能）/
            'gap'（最終が発走から max_lag 超前）/ 'ok'。
    lag_min: 最終 snapshot が発走の何分前か（post 以降取得なら負）。none/bad_ts は None。
    post 直前に日跨ぎは無い前提（JST 日中）だが、深夜帯の異常データで lag が極端な負になった場合も
    「post 以降に取れている」とみなし ok（取りこぼしではない）。
    """
    if not n_snaps:
        return {"status": "none", "lag_min": None}
    if last_snap_min is None:
        # snapshot はあるが時刻が読めない＝取りこぼし(none)ではなくデータ品質問題。混同させない。
        return {"status": "bad_ts", "lag_min": None}
    lag = post_min - last_snap_min
    status = "gap" if lag > max_lag_min else "ok"
    return {"status": status, "lag_min": lag}


def build_coverage(rows, max_lag_min):
    """(race_id, venue, race_num, post_time, last_fetched_at, n_snaps) 行 → レース別判定の list。

    post_time が不正/空のレースは判定不能として除外（カードはあるが発走時刻未取得）。
    """
    out = []
    for rid, venue, rnum, post_time, last_at, n_snaps in rows:
        post_min = hhmm_to_min(post_time) if post_time else None
        if post_min is None:
            continue
        last_min = fetched_at_to_jst_min(last_at) if last_at else None
        ev = classify(post_min, last_min, n_snaps, max_lag_min)
        out.append({"race_id": rid, "venue": venue, "race_num": int(rnum),
                    "post_time": post_time, "n_snaps": n_snaps, **ev})
    out.sort(key=lambda r: (r["venue"], r["race_num"]))
    return out


# --- 入力ロード（DB or 外部 TSV） ---
def _psql_dump(db_url, date):
    """race_cards × race_odds_snapshots を集計して 1 レース 1 行の TSV で返す。

    date は SQL リテラルへ補間するため、呼び出し側検証に依存せず関数内でも YYYY-MM-DD を
    再検証する（多層防御。psql -c はプレースホルダを取れないので形式を厳格に固定した値だけ通す）。
    """
    if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", date):
        raise ValueError(f"date は YYYY-MM-DD のみ許可: {date!r}")
    # MAX(fetched_at) は ::text にして COALESCE の '' と型を揃える（fetched_at が将来 timestamptz でも
    # none レース＝MAX が NULL の行で `''::timestamptz` 不正リテラルにならない。現 TEXT では no-op）。
    sql = (
        "SELECT c.race_id, c.venue, c.race_num, COALESCE(c.post_time,''), "
        "       COALESCE(MAX(s.fetched_at)::text, ''), COUNT(DISTINCT s.fetched_at) "
        "FROM race_cards c "
        "LEFT JOIN race_odds_snapshots s ON s.race_id = c.race_id "
        f"WHERE c.date = '{date}' "
        "GROUP BY c.race_id, c.venue, c.race_num, c.post_time "
        "ORDER BY c.venue, c.race_num;"
    )
    # DB 到達不能時の無言ハングを避ける接続タイムアウト（keep_awake.sh と対称、URL を弄らず効く）。
    env = {**os.environ, "PGCONNECT_TIMEOUT": os.environ.get("PGCONNECT_TIMEOUT", "5")}
    out = subprocess.run(
        ["psql", db_url, "-tA", "-F", "\t", "-c", sql],
        capture_output=True, text=True, check=True, env=env,
    )
    return out.stdout


def parse_rows(tsv_text):
    """TSV を (race_id, venue, race_num, post_time, last_fetched_at, n_snaps) の list へ。"""
    rows = []
    for line in tsv_text.splitlines():
        if not line.strip():
            continue
        cells = line.split("\t")
        if len(cells) != 6:
            print(f"[warn] 想定外の列数 {len(cells)} をスキップ: {line[:80]}", file=sys.stderr)
            continue
        rid, venue, rnum, post_time, last_at, n = cells
        # race_num・n_snaps とも数値必須（外部 --rows-tsv の破損行で build_coverage の int() が
        # 落ちないよう入口で弾く。DB 経路は整数列なので実害は外部入力のみ）。
        try:
            n_snaps = int(n)
            int(rnum)
        except ValueError:
            print(f"[warn] race_num/n_snaps が数値でない行をスキップ: {line[:80]}", file=sys.stderr)
            continue
        rows.append((rid, venue, rnum, post_time, last_at, n_snaps))
    return rows


def print_report(cov, date, max_lag_min):
    print(f"=== snapshot カバレッジ（{date}・最終 snapshot が発走 {max_lag_min} 分前以内なら ok） ===")
    print("  場 R    発走   snap数  最終→発走  判定")
    label = {"ok": "✅ok", "gap": "⚠gap", "none": "❌none", "bad_ts": "⚠bad_ts"}
    for r in cov:
        lag = r["lag_min"]
        if lag is None:
            lag_s = "  --"
        elif lag < 0:
            lag_s = f"post後{-lag}分"  # post 以降に取得（取りこぼしではない）
        else:
            lag_s = f"{lag:>3}分前"
        print(f"  {r['venue']:<4}{r['race_num']:>2}R {r['post_time']:>5} "
              f"{r['n_snaps']:>4}本 {lag_s:>9}  {label[r['status']]}")
    n = len(cov)
    if n == 0:
        print("\n対象レースなし（post_time 入りカードが無い）")
        return 0
    ok = sum(1 for r in cov if r["status"] == "ok")
    gap = sum(1 for r in cov if r["status"] == "gap")
    none = sum(1 for r in cov if r["status"] == "none")
    bad_ts = sum(1 for r in cov if r["status"] == "bad_ts")
    print(f"\n  対象 {n}R  /  ✅ok {ok} ({ok / n * 100:.0f}%)  "
          f"⚠gap {gap}  ❌none {none}  ⚠bad_ts {bad_ts}")
    bad_total = gap + none + bad_ts
    if bad_total:
        bad = [f"{r['venue']}{r['race_num']}R" for r in cov if r["status"] != "ok"]
        print(f"  要確認 {bad_total}R: {' '.join(bad)}")
        print("  → 発走直前 snapshot の欠落/時刻不明。原因は Mac スリープ/不在等（#264）。過去オッズは復元不能。")
    return bad_total


def main(argv=None):
    ap = argparse.ArgumentParser(description="snapshot 取りこぼし検知（#264）")
    ap.add_argument("--date", help="開催日 YYYY-MM-DD（既定: JST 今日）")
    ap.add_argument("--max-lag-min", type=int, default=10,
                    help="最終 snapshot が発走の何分前までを ok とするか（既定 10）")
    ap.add_argument("--db-url", default=os.environ.get("PADDOCK_DB_URL", DEFAULT_DB_URL))
    ap.add_argument("--rows-tsv", help="集計済み TSV を外部供給（指定時 DB を引かない）")
    ap.add_argument("--fail-on-gap", action="store_true",
                    help="gap/none/bad_ts が 1 件でもあれば exit 1（CI/監視用）")
    args = ap.parse_args(argv)

    if args.rows_tsv:
        from pathlib import Path
        tsv = Path(args.rows_tsv).read_text()
        date = args.date or "(tsv)"
    else:
        from datetime import datetime
        from zoneinfo import ZoneInfo
        date = args.date or datetime.now(ZoneInfo("Asia/Tokyo")).strftime("%Y-%m-%d")
        if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", date):
            ap.error(f"--date は YYYY-MM-DD: {date}")
        try:
            tsv = _psql_dump(args.db_url, date)
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            print(f"DB 取得に失敗（psql/接続）: {e}", file=sys.stderr)
            sys.exit(1)

    cov = build_coverage(parse_rows(tsv), args.max_lag_min)
    bad = print_report(cov, date, args.max_lag_min)
    if args.fail_on_gap and bad:
        sys.exit(1)


if __name__ == "__main__":
    main()
