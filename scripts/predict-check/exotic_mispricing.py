#!/usr/bin/env python3
"""#314 エキゾ（馬連/3連複/馬単）ミスプライス収穫の検証（#272 配下）。

ADR 0053（#309）で「純モデル・学習モデルは単勝市場に勝てない（市場が過去走 fundamental を包含）」と
確定し、単勝で当てに行く筋は閉じた。残るエッジ候補は「群衆が組合せ確率を正しく合成できず
ミスプライスが残りやすい派生市場」（Benter/Ziemba のエッジ源泉）。本スクリプトは新規モデルを作らず、
**リーク無しの市場ブレンド単勝確率（production α=0.2, as-of）を Plackett-Luce/Harville でエキゾ組合せ
確率に展開 → 実エキゾオッズと突合し、控除率を net で抜けて +EV になる券種・帯があるか**を測る。

入力（結果リーク無し。model_win は as-of、オッズは EV 選抜専用で清算には使わない）:
  --dump         analyze backtest --dump-features TSV（model_win = α=0.2 ブレンド後・as-of・Σ=1）
  --exotic-odds  bt_exotic_odds.tsv（quinella/trio/exacta。gen_win_backtest_data.sh 生成。DB race_odds
                 の保存＝最終盤面オッズ。EV 選抜のみに使い清算には使わないので結果リークは無い）
  --results-dir  res_<nkid>.html（実配当。清算に使う＝EV 選抜は DB オッズで循環回避 #250）
  --races        bt_races.tsv（頭数・日付・nkid メタ）

合成確率・パース・配当は umaren_backtest.py の関数を再利用する（import U）。本スクリプトは
「組合せ単位の EV・実配当清算・券種/オッズ帯/頭数帯バケット集計・baseline・5 日別安定性」を足す。

留保: 検証母集団は小さい（エキゾオッズ ∩ 結果あり = 数十〜80R 規模、ワイドは過去データ不足で除外）。
結論は過学習留保つきの「兆候チェック」。5 日すべてで大きく安定した +EV が出ない限り本番接続は推奨しない
（ADR 0045/0053 と同じ規律）。JRA 控除率は 3 連系 ~27.5% / 馬連 ~22.5% で net バーは単勝より高い。
"""
import argparse
import statistics
from pathlib import Path

import umaren_backtest as U

# DB bet_type → netkeiba result.html の払戻キー（馬連は netkeiba 上 Umaren＝pay["umaren"]）。
PAY_KEY = {"quinella": "umaren", "trio": "trio", "exacta": "exacta"}
BET_TYPES = ("quinella", "trio", "exacta")


def parse_dump(path):
    """analyze backtest --dump-features TSV -> {race_id: {horse_num: model_win(0-1)}}。

    model_win は as-of（date<D）でリーク無しの市場ブレンド(α=0.2)最終 win 確率（レース内 Σ=1）。
    列はヘッダ名で引く（列順変更に頑健）。
    """
    lines = Path(path).read_text().splitlines()
    if not lines:
        return {}
    header = lines[0].split("\t")
    idx = {name: i for i, name in enumerate(header)}
    missing = [c for c in ("race_id", "horse_num", "model_win") if c not in idx]
    if missing:
        raise ValueError(f"--dump に必須列がありません: {missing}（ヘッダ: {header}）")
    ri, hi, wi = idx["race_id"], idx["horse_num"], idx["model_win"]
    out = {}
    for line in lines[1:]:
        if not line.strip():
            continue
        c = line.split("\t")
        out.setdefault(c[ri], {})[int(c[hi])] = float(c[wi])
    return out


def synth_prob(bet_type, probs, combo):
    """win 確率 dict と組合せから Plackett-Luce/Harville で券種の的中確率を合成する。

    combo は quinella/trio が frozenset、exacta が (1着,2着) タプル。probs に居ない馬（取消等）を
    含む組合せは None を返す（呼び出し側でスキップ）。
    """
    if bet_type == "exacta":
        a, b = combo
        if a not in probs or b not in probs:
            return None
        return U.p_exacta(probs, a, b)
    members = tuple(combo)
    if any(m not in probs for m in members):
        return None
    if bet_type == "quinella":
        if len(members) != 2:  # 退化キー（例 '9-9'）への防御。parse_result はサイズガード有りだが parse_exotic は無い。
            return None
        a, b = members
        return U.p_top2_set(probs, a, b)
    if bet_type == "trio":
        if len(members) != 3:
            return None
        return U.p_top3_set(probs, members)
    raise ValueError(f"未知の bet_type: {bet_type}")


def collect_bets(dump, exotic, races, results_dir, bet_types):
    """各レース・各組合せの投票候補レコードを作る。

    レコード: dict(pid, date, bet_type, combo, p, odds, ev, n_horses, hit, payout)。
      p     … 合成的中確率（Plackett-Luce）
      odds  … DB エキゾオッズ（EV 選抜に使う。清算には使わない＝循環回避 #250）
      ev    … p*odds - 1
      hit   … 実結果で的中したか（res_<nkid>.html の実配当で判定）
      payout… 的中時の実配当（円/100円）。未的中は 0。
    """
    bets = []
    for r in races:
        pid, date, nk = r["pid"], r["date"], r["nk"]
        probs = dump.get(pid)
        ex = exotic.get(pid)
        if not probs or not ex:
            continue
        resf = Path(results_dir) / f"res_{nk}.html"
        if not resf.exists():
            continue
        _top3, pay = U.parse_result(resf)
        n_horses = len(probs)
        for bt in bet_types:
            odds_map = ex.get(bt, {})
            paid = pay.get(PAY_KEY[bt], {})
            for combo, odds in odds_map.items():
                p = synth_prob(bt, probs, combo)
                if p is None or odds <= 0:
                    continue
                payout = paid.get(combo, 0)
                bets.append(dict(
                    pid=pid, date=date, bet_type=bt, combo=combo,
                    p=p, odds=odds, ev=p * odds - 1.0,
                    n_horses=n_horses, hit=payout > 0, payout=payout,
                ))
    return bets


def roi(bets):
    """賭金 100 円/点で realized ROI(%) と的中率(%) を返す（未的中は payout=0）。"""
    if not bets:
        return float("nan"), float("nan"), 0
    stake = 100 * len(bets)
    ret = sum(b["payout"] for b in bets)
    hits = sum(1 for b in bets if b["hit"])
    return ret / stake * 100, hits / len(bets) * 100, len(bets)


def _fmt_roi(label, bets, width=28):
    r, hr, n = roi(bets)
    ev = statistics.fmean([b["ev"] for b in bets]) if bets else float("nan")
    print(f"{label:<{width}} n={n:>5}  予測EV平均={ev:>+6.2f}  実現ROI={r:>6.1f}%  的中率={hr:>5.1f}%")


def report_by_bet_type(bets, thetas):
    """券種別: 全組合せ均等（baseline）と 合成EV≥θ 選抜の realized ROI。

    「全組合せ均等」= その券種の生 takeout ドラッグ（市場効率の下限）。合成EV≥θ がこれを上回れば
    合成確率が市場を超える情報を持つ兆候。θ を上げるほど ROI が上がる（＝EV が正直な選別）かも見る。
    """
    print("=== 券種別: baseline（全組合せ均等）vs 合成EV≥θ 選抜 ===")
    for bt in sorted({b["bet_type"] for b in bets}):
        sub = [b for b in bets if b["bet_type"] == bt]
        print(f"[{bt}]")
        _fmt_roi("  baseline(全組合せ均等)", sub)
        for th in thetas:
            _fmt_roi(f"  合成EV>={th:+.2f}", [b for b in sub if b["ev"] >= th])
        print()


def report_calibration(bets, edges):
    """予測 EV 帯 → 実現 ROI の較正（券種別）。#263 型の逆予測（高 EV ほど実現 ROI が低い）でないか。

    Spearman(予測EV, 個別リターン倍率) も出す（>0 なら EV が実現を正しく順位づけ）。個別リターン倍率 =
    payout/100（未的中 0）。edges は EV の境界（昇順）。
    """
    print("=== 予測EV帯 → 実現ROI 較正（券種別。逆予測性チェック）===")
    for bt in sorted({b["bet_type"] for b in bets}):
        sub = [b for b in bets if b["bet_type"] == bt]
        print(f"[{bt}]  Spearman(予測EV, リターン倍率)="
              f"{U.spearman([b['ev'] for b in sub], [b['payout'] / 100.0 for b in sub]):+.3f}")
        bounds = [-float("inf")] + list(edges) + [float("inf")]
        for lo, hi in zip(bounds[:-1], bounds[1:]):
            band = [b for b in sub if lo <= b["ev"] < hi]
            lbl = f"  EV[{lo:+.2f},{hi:+.2f})" if lo != -float("inf") and hi != float("inf") \
                else (f"  EV<{hi:+.2f}" if lo == -float("inf") else f"  EV>={lo:+.2f}")
            _fmt_roi(lbl, band, width=22)
        print()


def report_bucket(bets, key_fn, title, order=None):
    """任意キー（頭数帯・オッズ帯等）で券種横断バケット集計。合成EV≥0 選抜 ROI で並べる。"""
    print(f"=== {title}（合成EV>=0 選抜の realized ROI）===")
    sel = [b for b in bets if b["ev"] >= 0.0]
    keys = sorted({key_fn(b) for b in sel}, key=(order or (lambda k: k)))
    for k in keys:
        _fmt_roi(f"  {k}", [b for b in sel if key_fn(b) == k], width=22)
    print()


def report_by_day(bets, thetas):
    """開催日別（安定性）: 合成EV≥θ*（代表 θ）の realized ROI。単一日依存でないか。"""
    th = thetas[len(thetas) // 2] if thetas else 0.0
    print(f"=== 開催日別 安定性（合成EV>={th:+.2f} 選抜。単一日依存チェック）===")
    for d in sorted({b["date"] for b in bets}):
        _fmt_roi(f"  {d}", [b for b in bets if b["date"] == d and b["ev"] >= th], width=22)
    print()


def odds_band(odds):
    """組合せオッズ帯（人気帯の代理）。"""
    for hi in (5, 10, 20, 50, 100):
        if odds < hi:
            return f"<{hi}"
    return ">=100"


ODDS_ORDER = {"<5": 0, "<10": 1, "<20": 2, "<50": 3, "<100": 4, ">=100": 5}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dump", required=True, help="analyze backtest --dump-features TSV（as-of model_win）")
    ap.add_argument("--exotic-odds", required=True, help="bt_exotic_odds.tsv（quinella/trio/exacta）")
    ap.add_argument("--results-dir", required=True, help="res_<nkid>.html 保存 dir（実配当）")
    ap.add_argument("--races", required=True, help="bt_races.tsv（頭数・日付・nkid）")
    ap.add_argument("--bet-types", default="quinella",
                    help="検証券種（カンマ区切り。既定は最小構成の馬連。例 quinella,trio,exacta）")
    ap.add_argument("--ev-grid", default="0.0,0.2,0.5,1.0", help="合成EV≥θ 選抜の θ グリッド")
    ap.add_argument("--ev-bucket-edges", default="-0.5,-0.2,0.0,0.5,1.0", help="較正バケットの EV 境界")
    args = ap.parse_args()

    bet_types = [x.strip() for x in args.bet_types.split(",") if x.strip()]
    bad = [b for b in bet_types if b not in BET_TYPES]
    if bad:
        ap.error(f"--bet-types は {BET_TYPES} のみ: {bad}")
    thetas = [float(x) for x in args.ev_grid.split(",")]
    edges = [float(x) for x in args.ev_bucket_edges.split(",")]

    dump = parse_dump(args.dump)
    exotic = U.parse_exotic(args.exotic_odds)
    races = U.parse_races(args.races)

    bets = collect_bets(dump, exotic, races, args.results_dir, bet_types)
    n_races = len({b["pid"] for b in bets})
    print(f"# エキゾ・ミスプライス検証（#314）: 券種={bet_types} / 評価レース={n_races}R / "
          f"投票候補={len(bets)}点\n")
    if not bets:
        print("投票候補が 0。入力（--dump / --exotic-odds / --results-dir）と券種を確認してください。")
        return

    report_by_bet_type(bets, thetas)
    report_calibration(bets, edges)
    report_bucket(bets, lambda b: b["n_horses"], "頭数帯別",
                  order=lambda k: k)
    report_bucket(bets, lambda b: odds_band(b["odds"]), "組合せオッズ帯別（人気帯）",
                  order=lambda k: ODDS_ORDER.get(k, 99))
    report_by_day(bets, thetas)


if __name__ == "__main__":
    main()
