#!/usr/bin/env python3
"""当日の全レースの予想 JSON を生成する（ingest-predictions 入力）。

api-server の確率推定（market α=0.3 ブレンド）を本命の源として、勝率上位 5 頭に
◎○▲△☆ を付与。jockey は card API、単勝オッズ/人気は DB から補う。bets はモデル確率
重み配分（¥5,000/レース）で生成する（ADR-0032）。+EV/−EV 判定は refresh_ev.sh で。

使い方:
    python3 gen_predictions.py <YYYY-MM-DD> | cargo run -p ingest-predictions

環境変数:
    PADDOCK_API_URL  api-server のベース URL（既定 http://127.0.0.1:8080）
    PADDOCK_DB_URL   Postgres 接続 URL（既定 postgres://paddock:paddock@127.0.0.1:5432/paddock）

出力: prediction JSON 配列を stdout へ。ingest-predictions にパイプする。
"""
import json
import os
import re
import subprocess
import sys
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from live_ev import BET_LABEL, build_bets  # noqa: E402

API = os.environ.get("PADDOCK_API_URL", "http://127.0.0.1:8080")
# host は localhost ではなく 127.0.0.1 に固定する（#212）。Colima は IPv4(127.0.0.1) のみ
# 公開しており、localhost が ::1 に先解決されると psql が別の postgres に当たって間欠失敗する。
# PADDOCK_DB_URL で上書きする場合も host は localhost を避け 127.0.0.1 を使うこと（同じ間欠失敗が再発する）。
DB_URL = os.environ.get("PADDOCK_DB_URL", "postgres://paddock:paddock@127.0.0.1:5432/paddock")
BLEND = "0.3"
BUDGET = 5000
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
    sorted_probs = sorted(pred["probabilities"], key=lambda p: -p["win_prob"])
    if not sorted_probs:  # 出走馬確率なし（障害等）はスキップ
        continue
    mark_of = {p["horse_num"]: MARKS[i] for i, p in enumerate(sorted_probs[:len(MARKS)])}
    horses = []
    for p in sorted_probs:
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
    # モデル確率（[0,1]）から買い目を生成（ADR-0032）。+EV フィルタなし: 判定は refresh_ev.sh で。
    # build_bets はスケール非依存（max/比率のみ使用）のため [0,1] のまま渡してよい。
    prob_dict = {p["horse_num"]: p["win_prob"] for p in sorted_probs}
    # sorted_probs 空ガードより後なので prob_dict は必ず 1 件以上。
    # _ax/_parts/_kon は現時点では不使用（将来の viewer 拡張で印・軸を表示する際に利用可）。
    try:
        _ax, _parts, _kon, race_bets = build_bets(prob_dict, BUDGET)
    except Exception as exc:  # noqa: BLE001 — 予期しない内部エラーは bets 空でフォールバック
        print(f"[warn] {rid}: build_bets 失敗、買い目スキップ: {exc}", file=sys.stderr)
        race_bets = []
    bets_json = [
        # combo は build_bets が返す tuple[int, ...] (昇順ソート済み)
        {"bet_type": BET_LABEL.get(kind, kind),
         "combination": "-".join(str(n) for n in combo),
         "amount": amt}
        for kind, combo, amt in race_bets
    ]
    preds.append({
        "date": card["date"],
        "venue": card["venue"],
        "race_num": card["race_num"],
        "title": f"{card['surface']} {card['distance']}m",
        "budget": BUDGET,
        "strategy_note": "モデル(市場α=0.3ブレンド)勝率上位に ◎○▲△☆。買い目はモデル確率重み配分(¥5,000)。+EV/−EV 判定は refresh_ev.sh で。",
        "horses": horses,
        "bets": bets_json,
    })

json.dump(preds, sys.stdout, ensure_ascii=False, indent=2)
print(f"\n# {len(preds)} races generated", file=sys.stderr)
