#!/usr/bin/env python3
"""当日の全レースの予想 JSON を生成する（Obsidian view 用, #34/ingest-predictions 入力）。

api-server の確率推定（market α=0.3 ブレンド）を本命の源として、勝率上位 5 頭に
◎○▲△☆ を付与。jockey は card API、単勝オッズ/人気は DB から補う。bets は付けない
（本命確認が目的。買い目はライブで変動するため別途。買い目生成は #201）。

使い方:
    python3 gen_predictions.py <YYYY-MM-DD> | cargo run -p ingest-predictions
    cargo run -p ingest-predictions -- --render   # pad MD を生成して web-viewer で見る

環境変数:
    PADDOCK_API_URL  api-server のベース URL（既定 http://127.0.0.1:8080）
    PADDOCK_DB_URL   Postgres 接続 URL（既定 postgres://paddock:paddock@localhost:5432/paddock）

出力: prediction JSON 配列を stdout へ。ingest-predictions にパイプする。
"""
import json
import os
import re
import subprocess
import sys
import urllib.request

API = os.environ.get("PADDOCK_API_URL", "http://127.0.0.1:8080")
DB_URL = os.environ.get("PADDOCK_DB_URL", "postgres://paddock:paddock@localhost:5432/paddock")
BLEND = "0.3"
MARKS = ["◎", "○", "▲", "△", "☆"]  # 勝率上位 5 頭へ

if len(sys.argv) < 2 or not re.fullmatch(r"\d{4}-\d{2}-\d{2}", sys.argv[1]):
    sys.exit("usage: gen_predictions.py <YYYY-MM-DD>")
DATE = sys.argv[1]


def get(url):
    with urllib.request.urlopen(url, timeout=20) as r:
        return json.load(r)


def fetch_all_odds(date):
    """当日の全レースの単勝オッズを 1 クエリで取得: {race_id: {horse_num: (odds, popularity)}}。

    race_id をループ内で補間せず、`psql -v date=...` の変数束縛（`:'date'` は安全にクォート展開）
    で 1 回だけ引く。SQL 文字列補間の禁止（rules/sql/queries.md）と N+1 回避を両立する。
    変数展開は `-c` では効かないため、SQL は stdin 経由で psql に渡す。
    """
    sql = ("SELECT o.race_id, o.combination_key, o.odds, o.popularity "
           "FROM race_odds o JOIN race_cards c ON c.race_id = o.race_id "
           "WHERE c.date = :'date' AND o.bet_type = 'win';")
    out = subprocess.run(
        ["psql", DB_URL, "-tA", "-F", "\t", "-v", f"date={date}"],
        input=sql, capture_output=True, text=True, check=True,
    ).stdout
    d = {}
    for line in out.splitlines():
        if not line.strip():
            continue
        rid, num, odds, pop = line.split("\t")
        d.setdefault(rid, {})[int(num)] = (float(odds), int(pop) if pop else None)
    return d


# races 一覧と odds は冒頭で 1 回ずつ取得し、失敗（api-server/DB 未起動）は fail-fast する。
# 個別レースの 4xx/5xx（障害レース等）だけをループ内で skip し、全体停止と区別する。
try:
    races = get(f"{API}/api/races?date={DATE}")["races"]
except Exception as exc:
    sys.exit(f"races 一覧の取得に失敗（api-server 未起動? {API}）: {exc}")
all_odds = fetch_all_odds(DATE)

preds = []
for r in races:
    rid = r["race_id"]
    try:
        pred = get(f"{API}/api/races/{rid}/prediction?blend_alpha={BLEND}")
        card = get(f"{API}/api/races/{rid}")
    except Exception as exc:  # noqa: BLE001 — 障害レース等は API が 4xx/5xx を返す。skip して継続。
        print(f"[skip] {rid}: {exc}", file=sys.stderr)
        continue
    jockey = {ent["horse_num"]: ent.get("jockey", "") for ent in card["entries"]}
    odds = all_odds.get(rid, {})
    # 勝率降順で印を決める。
    probs = sorted(pred["probabilities"], key=lambda p: -p["win_prob"])
    mark_of = {p["horse_num"]: MARKS[i] for i, p in enumerate(probs[:len(MARKS)])}
    horses = []
    for p in probs:
        n = p["horse_num"]
        o, pop = odds.get(n, (None, None))
        h = {
            "horse_num": n,
            "horse_name": p["horse_name"],
            "jockey": jockey.get(n, ""),
            "win_prob": round(p["win_prob"] * 100, 1),
            "place_prob": round(p["place_prob"] * 100, 1),
            "show_prob": round(p["show_prob"] * 100, 1),
        }
        if n in mark_of:
            h["mark"] = mark_of[n]
        if o is not None:
            h["win_odds"] = o
        if pop is not None:
            h["popularity"] = pop
        horses.append(h)
    preds.append({
        "date": card["date"],
        "venue": card["venue"],
        "race_num": card["race_num"],
        "title": f"{card['surface']} {card['distance']}m",
        "strategy_note": "モデル(市場α=0.3ブレンド)の勝率上位に ◎○▲△☆。本命確認用（買い目は別途ライブEVで判断）。",
        "horses": horses,
    })

json.dump(preds, sys.stdout, ensure_ascii=False, indent=2)
print(f"\n# {len(preds)} races generated", file=sys.stderr)
