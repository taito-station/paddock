"""snapshot ベースの +EV 発生率レポート（#248）.

`race_odds_snapshots`（発走直前オッズの時系列アーカイブ, #232）に当て、開催日×レース単位で
「そのレースが（いずれかの snapshot 時点で）+EV(ROI≥100%) だったか」を後追い集計する。
年間の +EV 発生率（買い方ルール ROI≥100% が機能しているかの検証）の土台。

買い方・ROI ロジックは `live_ev.py` の `build_bets` / `race_roi` を **そのまま再利用**する
（複製しない）。本スクリプトの責務はデータ源を「live の race_odds」から「snapshots の時系列」に
差し替え、各 race の全 snapshot 時点を走査して ever/final の +EV を判定することだけ。

データ源:
  - オッズ: race_odds_snapshots（単勝・馬連・3連複・ワイド）。1 回の取得は単一 fetched_at で
            全券種が書かれる（save_race_odds）ため「1 snapshot 時点 = 1 fetched_at」で扱う。
            ワイドは low/high の幅で保存されるので mid=(low+high)/2 を採る（live_ev のワイド意味論）。
  - model 勝率: `analyze predict --blend-alpha <α>`（既定 0.2＝本番）。
                analyze predict は現行 race_odds の単勝を α ブレンドに読むため、**当日中に走らせれば
                final snapshot と一致し忠実**。後日実行は α 由来の僅差が出うる（オッズ自体は
                snapshots から忠実に取るため EV の支配項は忠実）。--pred-tsv で外部供給して固定可。

使い方:
    # 単日（実 DB を引く）
    python3 snapshot_ev_report.py --from 2026-06-27

    # 期間（年間集計）
    python3 snapshot_ev_report.py --from 2026-01-01 --to 2026-12-31 --budget 5000

    # テスト/再現用に外部 TSV を注入（DB/analyze に触らない）
    python3 snapshot_ev_report.py --snapshots-tsv snaps.tsv --pred-tsv preds.tsv

入力 TSV（--snapshots-tsv）形式（タブ区切り・1 行 1 オッズ）:
    race_id  date  venue  race_num  surface  distance  bet_type  combination_key  odds  odds_high  fetched_at
入力 TSV（--pred-tsv）形式:
    race_id  horse_num  model_pct
"""
import argparse
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

import live_ev as L

# snapshots から集計に使う券種（live_ev の全3券種 ROI に必要な分）。単勝は出走馬の確定に使う。
WANT_BET_TYPES = ("win", "quinella", "trio", "wide")

DEFAULT_DB_URL = "postgres://paddock:paddock@127.0.0.1:5432/paddock"
# analyze predict の確率行: 「馬番 馬名 勝率%」。live_ev.parse_pred と同じ規約。
PRED_LINE_RE = re.compile(r"\s*(\d+)\s+\S+\s+([\d.]+)%")


def _combo(key: str) -> tuple[int, ...]:
    """combination_key("1-2" / "1-2-3") を昇順 tuple へ。順序付き("a>b")は対象外なので呼ばない。"""
    return tuple(sorted(int(x) for x in key.split("-")))


def group_snapshots(rows):
    """snapshot 行（dict の iterable）を race_id → レース情報へ畳む（純関数・DB 非依存）。

    返り値: race_id -> {
        "date","venue","race_num","surface","distance",
        "times": { fetched_at: {"win":{n:odds}, "quinella":{tuple:odds},
                                "trio":{tuple:odds}, "wide":{tuple:mid}} },
    }
    ワイドは low(odds)/high(odds_high) から mid=(low+high)/2 を採る（live_ev と同一意味論）。
    odds_high 欠落のワイドは構造異常としてスキップ（mid を出せない）。
    """
    races = {}
    for r in rows:
        bt = r["bet_type"]
        if bt not in WANT_BET_TYPES:
            continue
        rid = r["race_id"]
        race = races.get(rid)
        if race is None:
            race = races[rid] = {
                "date": r["date"],
                "venue": r["venue"],
                "race_num": int(r["race_num"]),
                "surface": r["surface"],
                "distance": int(r["distance"]) if r["distance"] else 0,
                "times": defaultdict(lambda: {"win": {}, "quinella": {}, "trio": {}, "wide": {}}),
            }
        books = race["times"][r["fetched_at"]]
        odds = float(r["odds"])
        if bt == "win":
            books["win"][int(r["combination_key"])] = odds
        elif bt == "wide":
            hi = r["odds_high"]
            if hi in (None, "", "\\N"):
                continue  # mid を出せない異常行は捨てる
            books["wide"][_combo(r["combination_key"])] = (odds + float(hi)) / 2.0
        else:  # quinella / trio
            books[bt][_combo(r["combination_key"])] = odds
    return races


def eval_race(probs, times, budget):
    """1 レースを全 snapshot 時点で評価し ever/final の +EV を判定する（純関数・live_ev 再利用）。

    probs: {horse_num: model_pct}（出走馬で絞り込み済み）。
    times: {fetched_at: books}（group_snapshots の "times"）。
    買い目 bets は probs と budget のみで決まりオッズに依存しないため 1 度だけ組み、
    各 fetched_at のオッズで ROI を出す（オッズ変動だけが ROI を動かす）。

    返り値 dict: roi 系列が空（出走馬<3 等で買い目が組めない）なら None。
    """
    if len(probs) < 3:
        return None
    _ax, _parts, kon, bets = L.build_bets(probs, budget)
    if not bets:
        return None
    series = []  # (fetched_at, roi)
    for at in sorted(times):
        b = times[at]
        roi, _stake, _missing = L.race_roi(probs, bets, b["wide"], b["quinella"], b["trio"])
        series.append((at, roi))
    if not series:
        return None
    final_at, final_roi = series[-1]
    best_at, best_roi = max(series, key=lambda t: t[1])
    return {
        "konsen": kon,
        "n_times": len(series),
        "final_at": final_at,
        "final_roi": final_roi,
        "best_at": best_at,
        "best_roi": best_roi,
        "ever_pos": best_roi >= 100.0,
        "final_pos": final_roi >= 100.0,
    }


# --- 入力ロード（DB or 外部 TSV） ---
def _psql_dump_snapshots(db_url, date_from, date_to):
    """期間内の snapshot を race_cards と join して TSV 文字列で返す。"""
    sql = (
        "SELECT s.race_id, c.date, c.venue, c.race_num, c.surface, c.distance, "
        "       s.bet_type, s.combination_key, s.odds, COALESCE(s.odds_high::text,''), s.fetched_at "
        "FROM race_odds_snapshots s "
        "JOIN race_cards c ON c.race_id = s.race_id "
        f"WHERE c.date BETWEEN '{date_from}' AND '{date_to}' "
        "  AND s.bet_type IN ('win','quinella','trio','wide') "
        "ORDER BY s.race_id, s.fetched_at;"
    )
    out = subprocess.run(
        ["psql", db_url, "-tA", "-F", "\t", "-c", sql],
        capture_output=True, text=True, check=True,
    )
    return out.stdout


_SNAP_COLS = ["race_id", "date", "venue", "race_num", "surface", "distance",
              "bet_type", "combination_key", "odds", "odds_high", "fetched_at"]


def load_snapshot_rows(tsv_text):
    """snapshot TSV 文字列を dict 行の list へ。"""
    rows = []
    for line in tsv_text.splitlines():
        if not line.strip():
            continue
        cells = line.split("\t")
        if len(cells) != len(_SNAP_COLS):
            print(f"[warn] 想定外の列数 {len(cells)} をスキップ: {line[:80]}", file=sys.stderr)
            continue
        rows.append(dict(zip(_SNAP_COLS, cells)))
    return rows


def load_pred_tsv(path):
    """--pred-tsv（race_id\\thorse_num\\tmodel_pct）を race_id -> {horse_num: pct} へ。"""
    preds = defaultdict(dict)
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        rid, num, pct = line.split("\t")
        preds[rid][int(num)] = float(pct)
    return preds


def model_probs_via_analyze(race_id, alpha, analyze_bin, db_url):
    """analyze predict を 1 レース回して {horse_num: model_pct} を返す（取得失敗時は空 dict）。"""
    env = dict(os.environ, PADDOCK_DB_URL=db_url)
    try:
        out = subprocess.run(
            [analyze_bin, "predict", race_id, "--blend-alpha", str(alpha)],
            capture_output=True, text=True, env=env, check=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[warn] analyze predict 失敗 {race_id}: {e}", file=sys.stderr)
        return {}
    probs = {}
    for line in out.stdout.splitlines():
        m = PRED_LINE_RE.match(line)
        if m:
            probs[int(m.group(1))] = float(m.group(2))
    return probs


def build_report(races, preds_by_race, budget):
    """races（group_snapshots 出力）と probs を突き合わせ、レース別評価の list を返す。"""
    results = []
    for rid, race in races.items():
        probs_all = preds_by_race.get(rid, {})
        if not probs_all:
            continue
        # 最終 snapshot の出走馬（win が取れた馬番）で絞る。取消馬を買い目に混ぜない。
        win_horses = set(race["times"][max(race["times"])]["win"])
        probs = {n: p for n, p in probs_all.items() if not win_horses or n in win_horses}
        ev = eval_race(probs, race["times"], budget)
        if ev is None:
            continue
        results.append({**ev, "race_id": rid, "date": race["date"],
                        "venue": race["venue"], "race_num": race["race_num"]})
    results.sort(key=lambda r: (r["date"], r["venue"], r["race_num"]))
    return results


def print_report(results, budget):
    """レース別行 ＋ 年間サマリ（総R数・ever/final の +EV 率・日別）を出力。"""
    print(f"=== snapshot +EV レポート（¥{budget:,}・全3券種ROI） ===")
    print("  日付        場 R   時点 ever  best  final  判定")
    for r in results:
        ev_flag = "✅" if r["ever_pos"] else "  "
        fn_flag = "✅+EV" if r["final_pos"] else " −EV"
        kon = "[混戦]" if r["konsen"] else "     "
        print(f"  {r['date']} {r['venue']:<4}{r['race_num']:>2}R {r['n_times']:>3}本 "
              f"{ev_flag}{r['best_roi']:>5.0f}% {r['final_roi']:>5.0f}% {fn_flag} {kon}")
    n = len(results)
    if n == 0:
        print("\n対象レースなし（snapshot or 予想が揃わない）")
        return
    ever = sum(1 for r in results if r["ever_pos"])
    final = sum(1 for r in results if r["final_pos"])
    print(f"\n=== 年間サマリ ===")
    print(f"  対象レース   : {n}")
    print(f"  ever +EV     : {ever}  ({ever / n * 100:.1f}%)  ← いずれかの snapshot 時点で ROI≥100%")
    print(f"  final +EV    : {final}  ({final / n * 100:.1f}%)  ← 最終 snapshot で ROI≥100%")
    by_date = defaultdict(lambda: [0, 0, 0])  # date -> [races, ever, final]
    for r in results:
        d = by_date[r["date"]]
        d[0] += 1
        d[1] += r["ever_pos"]
        d[2] += r["final_pos"]
    print("  日別:")
    for d in sorted(by_date):
        races_n, ever_n, final_n = by_date[d]
        print(f"    {d}: {races_n}R  ever {ever_n}  final {final_n}")


def main():
    ap = argparse.ArgumentParser(description="snapshot ベース +EV 発生率レポート（#248）")
    ap.add_argument("--from", dest="date_from", help="開始日 YYYY-MM-DD（DB 直引き時に必須）")
    ap.add_argument("--to", dest="date_to", help="終了日 YYYY-MM-DD（既定: --from と同じ＝単日）")
    ap.add_argument("--budget", type=int, default=5000, help="1レース予算（円, 既定5000）")
    ap.add_argument("--blend-alpha", type=float, default=0.2, help="analyze predict の α（既定0.2）")
    ap.add_argument("--db-url", default=os.environ.get("PADDOCK_DB_URL", DEFAULT_DB_URL))
    ap.add_argument("--analyze-bin", default=None,
                    help="paddock-analyze バイナリ（既定: target/release/paddock-analyze）")
    ap.add_argument("--snapshots-tsv", help="snapshot TSV を外部供給（指定時 DB を引かない）")
    ap.add_argument("--pred-tsv", help="model 勝率 TSV を外部供給（指定時 analyze を回さない）")
    args = ap.parse_args()

    # --- snapshot ロード ---
    if args.snapshots_tsv:
        tsv = Path(args.snapshots_tsv).read_text()
    else:
        if not args.date_from:
            ap.error("--from が必要（または --snapshots-tsv を指定）")
        date_to = args.date_to or args.date_from
        for d in (args.date_from, date_to):
            if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", d):
                ap.error(f"日付は YYYY-MM-DD: {d}")
        try:
            tsv = _psql_dump_snapshots(args.db_url, args.date_from, date_to)
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            print(f"snapshot の取得に失敗（psql/DB 接続）: {e}", file=sys.stderr)
            sys.exit(1)
    races = group_snapshots(load_snapshot_rows(tsv))
    if not races:
        print("対象 snapshot なし", file=sys.stderr)
        sys.exit(0 if args.snapshots_tsv else 1)

    # --- model 勝率ロード ---
    if args.pred_tsv:
        preds = load_pred_tsv(args.pred_tsv)
    else:
        analyze_bin = args.analyze_bin or str(
            Path(__file__).resolve().parents[2] / "target/release/paddock-analyze")
        if not os.access(analyze_bin, os.X_OK):
            print(f"analyze バイナリが見つかりません: {analyze_bin}\n"
                  f"  cargo build --release --bin paddock-analyze の後に再実行、"
                  f"または --pred-tsv で勝率を供給してください。", file=sys.stderr)
            sys.exit(1)
        preds = {}
        for rid in races:
            preds[rid] = model_probs_via_analyze(rid, args.blend_alpha, analyze_bin, args.db_url)

    print_report(build_report(races, preds, args.budget), args.budget)


if __name__ == "__main__":
    main()
