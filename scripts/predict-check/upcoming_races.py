#!/usr/bin/env python3
"""発走時刻ベースで「これから発走する」レースを絞り込む（ライブ EV 更新の対象決定用, #197）.

ライブ EV 更新ループを朝の早い時間帯から全レース対象で回すと、オッズが動かない時間帯に
netkeiba を無駄に叩くだけになる（IP ブロックのリスク, feedback_jra_fetch_pacing）。
本スクリプトは netkeiba race_list の発走時刻を使い、

  - 発走済みのレース（post < now）を除外
  - 発走まで window 分より先のレース（post > now + window）を除外

した「対象レース」だけを出力する。除外の2条件は同じ発走時刻ベースの判定で共通化する。

使い方:
    python3 upcoming_races.py YYYYMMDD [--window-min N] [--at HH:MM] [--all] [--venue CC ...]
例:
    python3 upcoming_races.py 20260621 --window-min 60       # 発走 60 分以内の未発走レース
    python3 upcoming_races.py 20260621 --all                 # 全レース（フィルタ無効・後方互換）
    python3 upcoming_races.py 20260621 --at 10:30            # 現在時刻を 10:30 とみなして判定（テスト/検証用）

出力は TAB 区切り 3 列（list_races.py と同形式）:
    1 列目: netkeiba 12桁 race_id
    2 列目: 発走時刻 HH:MM
    3 列目: paddock 内部 race_id（refresh_ev の DB 突合用）
"""
import argparse
import re
import sys
from datetime import datetime
from zoneinfo import ZoneInfo

from nk import parse_race_id, race_post_times

# netkeiba の発走時刻は JST 前提。システム TZ に依存して窓判定がずれないよう、現在時刻は
# 常に JST で評価する（cron/別ホスト運用で TZ が JST 以外でも正しく動かすため）。
JST = ZoneInfo("Asia/Tokyo")


def to_minutes(hhmm: str) -> int:
    """"HH:MM" を 0:00 からの経過分に変換する。"""
    h, m = hhmm.split(":")
    return int(h) * 60 + int(m)


def valid_hhmm(s: str) -> str:
    """argparse 用: --at の値を `HH:MM`（00:00〜23:59）に検証する。

    不正値（`25:00` / `10:99` / `10` / `abc` 等）を黙って受理すると窓判定を静かに誤らせるため、
    範囲込みで弾いて分かりやすいエラーにする（nk.py の date/venue 検証と対称）。
    """
    m = re.fullmatch(r"([01]?\d|2[0-3]):([0-5]\d)", s)
    if not m:
        raise argparse.ArgumentTypeError(f"HH:MM（00:00〜23:59）形式で指定してください: {s!r}")
    return s


def select_upcoming(post_times: dict, now_min: int, window_min: int):
    """発走時刻 dict {race_id: "HH:MM"} から「now 以降かつ now+window 以内」の race_id を返す.

    純粋関数（ネットワーク・時刻取得なし）でテスト可能にする。境界は両端含む
    （post == now はこれから発走＝対象、post == now+window もちょうど window 端で対象）。
    発走時刻順 → race_id 順で安定ソートして返す。

    時刻は同日内の「0:00 からの経過分」で比較し、日跨ぎは扱わない（中央競馬は日中開催で
    発走時刻が深夜域に来ないため。深夜域の `--at`/システム時刻では全レースが窓外になりうる）。
    """
    selected = [
        rid for rid, hhmm in post_times.items()
        if now_min <= to_minutes(hhmm) <= now_min + window_min
    ]
    return sorted(selected, key=lambda r: (to_minutes(post_times[r]), r))


def paddock_race_id(p) -> str:
    return f"{p['year']}-{p['round']}-{p['venue_slug']}-{p['day']}-{p['race_num']}R"


def main(argv=None):
    ap = argparse.ArgumentParser(description="発走時刻で対象レースを絞る (#197)")
    ap.add_argument("date", help="開催日 YYYYMMDD")
    ap.add_argument("--window-min", type=int, default=60,
                    help="発走まで何分以内を対象にするか（既定 60）")
    ap.add_argument("--at", metavar="HH:MM", type=valid_hhmm,
                    help="現在時刻の上書き（テスト/検証用。既定はシステム時刻）")
    ap.add_argument("--all", action="store_true",
                    help="フィルタを無効化し全レースを出力（後方互換）")
    ap.add_argument("--venue", nargs="*", metavar="CC",
                    help="場コードで絞る（例: 05 09）")
    args = ap.parse_args(argv)

    post_times = race_post_times(args.date, args.venue)

    if args.all:
        ids = sorted(post_times, key=lambda r: (to_minutes(post_times[r]), r))
    else:
        if args.at:
            now_min = to_minutes(args.at)
        else:
            now = datetime.now(JST)
            now_min = now.hour * 60 + now.minute
        ids = select_upcoming(post_times, now_min, args.window_min)

    for rid in ids:
        p = parse_race_id(rid)
        print(f"{rid}\t{post_times[rid]}\t{paddock_race_id(p)}")
    print(f"# {len(ids)} races (of {len(post_times)})", file=sys.stderr)


if __name__ == "__main__":
    main()
