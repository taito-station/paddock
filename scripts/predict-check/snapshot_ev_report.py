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

入力 TSV（--snapshots-tsv）形式（タブ区切り・1 行 1 オッズ・列順は _SNAP_COLS と一致）:
    race_id  date  venue  race_num  bet_type  combination_key  odds  odds_high  fetched_at
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
# 勝率は厳格に「整数 or 小数1個」に絞る（`[\d.]+` だと `1.2.3` 等の多重ドットも拾い float() で落ちる）。
PRED_LINE_RE = re.compile(r"\s*(\d+)\s+\S+\s+(\d+(?:\.\d+)?)%")


def _combo(key: str) -> tuple[int, ...]:
    """combination_key("1-2" / "1-2-3") を昇順 tuple へ。順序付き("a>b")は対象外なので呼ばない。"""
    return tuple(sorted(int(x) for x in key.split("-")))


def group_snapshots(rows):
    """snapshot 行（dict の iterable）を race_id → レース情報へ畳む（純関数・DB 非依存）。

    返り値: race_id -> {
        "date","venue","race_num",
        "times": { fetched_at: {"win":{n:odds}, "quinella":{tuple:odds},
                                "trio":{tuple:odds}, "wide":{tuple:mid}} },
    }
    ワイドは low(odds)/high(odds_high) から mid=(low+high)/2 を採る（live_ev と同一意味論）。
    odds_high 欠落のワイドは構造異常としてスキップ（mid を出せない）。
    値（odds や馬番）が数値化できない異常行は当該行のみ warn スキップ（レポート全体を止めない）。
    """
    races = {}
    for r in rows:
        bt = r["bet_type"]
        if bt not in WANT_BET_TYPES:
            continue
        rid = r["race_id"]
        try:
            # 先に全数値化を済ませてから race/time へ書く。これで破損行（非数値）が
            # defaultdict 経由で空の phantom snapshot 時点を生成し、最終時点を上書きするのを防ぐ。
            odds = float(r["odds"])
            if bt == "win":
                book, key, val = "win", int(r["combination_key"]), odds
            elif bt == "wide":
                hi = r["odds_high"]
                if hi in (None, "", "\\N"):
                    continue  # mid を出せない異常行は捨てる
                book, key, val = "wide", _combo(r["combination_key"]), (odds + float(hi)) / 2.0
            else:  # quinella / trio
                book, key, val = bt, _combo(r["combination_key"]), odds
            race = races.get(rid)
            if race is None:
                race = races[rid] = {
                    "date": r["date"],
                    "venue": r["venue"],
                    "race_num": int(r["race_num"]),  # 外部 TSV の非数値 race_num も行単位で弾く
                    "times": defaultdict(
                        lambda: {"win": {}, "quinella": {}, "trio": {}, "wide": {}}),
                }
            race["times"][r["fetched_at"]][book][key] = val
        except ValueError:
            print(f"[warn] 数値化できない snapshot 行をスキップ: "
                  f"{rid} {bt} {r['combination_key']} {r['odds']}", file=sys.stderr)
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
    series = []  # (fetched_at, roi, missing)
    # fetched_at は save_race_odds が rfc3339 UTC（+00:00 固定書式）で書くため辞書順=時系列順。
    # final=最遅 fetched_at をこの前提で取る（migration 20260625 のコメントと同一規約）。
    for at in sorted(times):
        b = times[at]
        roi, _stake, missing = L.race_roi(probs, bets, b["wide"], b["quinella"], b["trio"])
        series.append((at, roi, missing))
    if not series:  # times は group_snapshots で必ず 1 件以上。到達しない防御的ガード。
        return None
    final_at, final_roi, final_missing = series[-1]
    best_at, best_roi, _best_missing = max(series, key=lambda t: t[1])
    return {
        "konsen": kon,
        "n_times": len(series),
        "final_at": final_at,
        "final_roi": final_roi,
        # 最終 snapshot のオッズ欠落（賭金は分母に残り分子0）。>0 なら final_roi は過小評価で、
        # final +EV→−EV を黙って誤判定しうる（live_ev と同じ保守側）。print_report で ⚠ 可視化する。
        "final_missing": final_missing,
        "best_at": best_at,
        "best_roi": best_roi,
        "ever_pos": best_roi >= 100.0,
        "final_pos": final_roi >= 100.0,
    }


# --- 入力ロード（DB or 外部 TSV） ---
def _psql_dump_snapshots(db_url, date_from, date_to):
    """期間内の snapshot を race_cards と join して TSV 文字列で返す。

    date_* は SQL リテラルへ補間するため、呼び出し側検証に依存せず関数内でも YYYY-MM-DD を
    再検証する（多層防御）。psql -c の単発クエリはプレースホルダを取れないので、形式を厳格に
    固定した値だけを通す。
    """
    # [0-9] に固定（\d は Unicode 数字も通すため、ASCII 桁のみ許可して曖昧な値を早期に弾く）。
    for d in (date_from, date_to):
        if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", d):
            raise ValueError(f"日付は YYYY-MM-DD のみ許可: {d!r}")
    # 対象券種は WANT_BET_TYPES を単一情報源に IN リストへ展開（いずれも内部定数で注入リスク無し）。
    bet_in = ",".join(f"'{t}'" for t in WANT_BET_TYPES)
    sql = (
        "SELECT s.race_id, c.date, c.venue, c.race_num, "
        "       s.bet_type, s.combination_key, s.odds, COALESCE(s.odds_high::text,''), s.fetched_at "
        "FROM race_odds_snapshots s "
        "JOIN race_cards c ON c.race_id = s.race_id "
        f"WHERE c.date BETWEEN '{date_from}' AND '{date_to}' "
        f"  AND s.bet_type IN ({bet_in}) "
        "ORDER BY s.race_id, s.fetched_at;"
    )
    out = subprocess.run(
        ["psql", db_url, "-tA", "-F", "\t", "-c", sql],
        capture_output=True, text=True, check=True,
    )
    return out.stdout


_SNAP_COLS = ["race_id", "date", "venue", "race_num",
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
    """--pred-tsv（race_id\\thorse_num\\tmodel_pct）を race_id -> {horse_num: pct} へ。

    列数・数値の不正行は load_snapshot_rows と同様に warn スキップし、1 行の崩れでレポート全体を
    落とさない（非対称な握り潰しを避ける）。
    """
    preds = defaultdict(dict)
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        cells = line.split("\t")
        if len(cells) != 3:
            print(f"[warn] pred-tsv の想定外の列数 {len(cells)} をスキップ: {line[:80]}",
                  file=sys.stderr)
            continue
        rid, num, pct = cells
        try:
            preds[rid][int(num)] = float(pct)
        except ValueError:
            print(f"[warn] pred-tsv の数値化できない行をスキップ: {line[:80]}", file=sys.stderr)
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
    # 確率表は predict 出力の先頭側に固まって出る（馬番 馬名 勝率%…）。現状の analyze 出力では
    # 後続の診断行（馬単EV・穴馬1着確率など）は本 regex にマッチしないことを実出力で確認済みだが、
    # 将来出力が変わっても先に出た確率表行を診断行に上書きされないよう「先勝ち」で確定する。
    probs = {}
    for line in out.stdout.splitlines():
        m = PRED_LINE_RE.match(line)
        if m:
            probs.setdefault(int(m.group(1)), float(m.group(2)))
    return probs


def build_report(races, preds_by_race, budget):
    """races（group_snapshots 出力）と probs を突き合わせ、レース別評価の list を返す。"""
    results = []
    for rid, race in races.items():
        probs_all = preds_by_race.get(rid, {})
        if not probs_all:
            continue
        # 出走馬（= 有効な win オッズが付いた馬番）で絞り、取消馬を買い目に混ぜない。
        # 「win を持つ最新 snapshot」の win 集合を採る。最終 snapshot が win 欠落（部分キャプチャ）
        # でも win のある直近時点へフォールバックし、かつ和集合のように 1 日全体へ窓を広げないので
        # 朝に早期取消された馬（終盤 snapshot の win から外れる）を拾わない。odds>0 のみ採用し
        # odds=0.0 のプレースホルダを「出走」と誤認しない（CLAUDE.md 既知問題）。
        win_horses = set()
        for at in sorted(race["times"], reverse=True):
            w = {n for n, o in race["times"][at]["win"].items() if o > 0}
            if w:
                win_horses = w
                break
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
        # 最終 snapshot にオッズ欠落があると final_roi は過小評価＝−EV へ振れうるため ⚠ で警告。
        miss = " ⚠欠落" if r.get("final_missing") else ""
        print(f"  {r['date']} {r['venue']:<4}{r['race_num']:>2}R {r['n_times']:>3}本 "
              f"{ev_flag}{r['best_roi']:>5.0f}% {r['final_roi']:>5.0f}% {fn_flag} {kon}{miss}")
    n = len(results)
    if n == 0:
        print("\n対象レースなし（snapshot or 予想が揃わない）")
        return
    ever = sum(1 for r in results if r["ever_pos"])
    final = sum(1 for r in results if r["final_pos"])
    nmiss = sum(1 for r in results if r.get("final_missing"))
    # final 判定が信頼できるのは最終 snapshot にオッズ欠落が無いレースだけ。欠落レースは
    # final_roi が過小評価で −EV 側に倒れるため、欠落込みの率は capture 取りこぼし(#264)で
    # 下方バイアスする。判定可能母数（欠落除外）の率を正とし、欠落込みは参考で併記する。
    jud = [r for r in results if not r.get("final_missing")]
    njud = len(jud)
    final_judged = sum(1 for r in jud if r["final_pos"])
    print("\n=== 年間サマリ ===")
    print(f"  対象レース   : {n}")
    print(f"  ever +EV     : {ever}  ({ever / n * 100:.1f}%)  ← いずれかの snapshot 時点で ROI≥100%")
    if njud:
        print(f"  final +EV    : {final_judged}/{njud}  ({final_judged / njud * 100:.1f}%)  "
              f"← 最終 snapshot で ROI≥100%（欠落除外の判定可能母数。これを正とする）")
    print(f"  （参考）欠落込: {final}/{n}  ({final / n * 100:.1f}%)  ← 欠落レースを −EV 計上した下限値")
    # ever は朝の剥がれやすい +EV も拾うため楽観側の参考値。実際に張れたかは発走直前＝final で見る。
    print("  ※ ever は朝オッズ込みの参考値（朝の +EV は直前で剥がれやすい）。actionable は final。")
    if nmiss:
        print(f"  ⚠ 最終欠落   : {nmiss}  最終 snapshot にオッズ欠落があり final 判定不能（#264 で対策）")
    # 日別も headline と母数を揃える: final は欠落除外の「判定可能母数中の +EV」で出し、欠落数を併記。
    by_date = defaultdict(lambda: [0, 0, 0, 0])  # date -> [races, ever, final_judged, judged]
    for r in results:
        d = by_date[r["date"]]
        d[0] += 1
        d[1] += r["ever_pos"]
        if not r.get("final_missing"):
            d[3] += 1
            d[2] += r["final_pos"]
    print("  日別:")
    for d in sorted(by_date):
        races_n, ever_n, final_n, judged_n = by_date[d]
        miss_n = races_n - judged_n
        miss_s = f"  欠落 {miss_n}" if miss_n else ""
        print(f"    {d}: {races_n}R  ever {ever_n}  final {final_n}/{judged_n}{miss_s}")


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
            if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", d):
                ap.error(f"日付は YYYY-MM-DD: {d}")
        # 逆順は BETWEEN が空集合になり「対象なし」と紛れるため明示エラーにする（形式が同じなので
        # 辞書順比較＝日付比較）。
        if date_to < args.date_from:
            ap.error(f"--to は --from 以降にしてください: {args.date_from}..{date_to}")
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
        if args.snapshots_tsv:
            # snapshots は外部 TSV なのに勝率は DB の analyze に頼る取り違えを警告（両者ペア利用が前提）。
            print("[warn] --snapshots-tsv 指定だが --pred-tsv 未指定。勝率は DB の analyze predict "
                  "から取得します（TSV 内のレースが DB に無ければ全 drop で『対象なし』になります）。",
                  file=sys.stderr)
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
