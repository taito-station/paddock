#!/usr/bin/env python3
"""純モデル（α=1.0）の win/place/show 校正計測。

α＝モデル重み（`blended = α·model + (1−α)·market`, estimate.rs / ADR 0045・0052）で **α=1.0 が純モデル /
α=0.0 が純市場**。`--pure` には gen_pure_preds.py を α=1.0（既定）で生成した TSV を渡すこと。α=0 で
生成した TSV は純市場なので「純モデル校正」にならない（旧既定の取り違え。#303 で是正）。

gen_pure_preds.py が出した純モデル prob と netkeiba 実結果を突き合わせ、以下を出す:
  - モデル幅（各レースの win 最大−最小）の分布＝フラットさの定量化
  - top-1 的中（argmax win が勝つか）。市場本命（人気1位）をベースラインに併記
  - win / place / show の reliability（確率帯ごとの予測 vs 実測）と ECE。win は市場 implied も併記
  - 大穴過大評価（モデル高 win × 市場低人気の馬の実勝率）

`analyze backtest`（Brier/LogLoss/ROI の集約スコア）に無い「確率帯ごとの校正ズレ」を
可視化する補完ツール。モデルの校正改修（縮約 m × recency × form の retune 等, #286）の
前後比較に使う。

既知の制約: 複勝（show）の的中は一律 top3 着で採点する。JRA の複勝は出走 7 頭以下だと
2 着以内のため、小頭数レースでは show reliability がわずかに過大（実測↑）に出る。広窓
（多くは 8 頭以上）での相対比較が用途なので影響は軽微。

入力は umaren_backtest の parse_result / parse_winodds を再利用。

使い方:
    python3 scripts/predict-check/calibration.py \
        --pure /tmp/bt252/pure_preds.tsv --results-dir /tmp/bt252 \
        --winodds /tmp/bt252/bt_winodds.tsv
"""
import argparse
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import umaren_backtest as ub  # noqa: E402

WIN_BINS = [0.0, 0.02, 0.05, 0.10, 0.20, 0.40, 1.01]
SHOW_BINS = [0.0, 0.10, 0.20, 0.30, 0.45, 0.60, 1.01]
PLACE_BINS = [0.0, 0.05, 0.12, 0.20, 0.30, 0.45, 1.01]


def load_pure(path):
    """pure_preds.tsv -> ({slug: {num: (win,place,show)}}, {slug: nk12}).

    キー slug は paddock のレース id（例 `2026-3-kyoto-11-10R`）。netkeiba 12 桁 nk12 とは別物で、
    umaren_backtest が parse_winodds でキーに使う id（同モジュールでは pid と呼ぶ）と同一。
    """
    preds, nk = {}, {}
    with open(path) as f:
        next(f, None)  # header（空ファイルでも StopIteration を出さない）
        for line in f:
            slug, nk12, num, win, place, show = line.rstrip("\n").split("\t")
            preds.setdefault(slug, {})[int(num)] = (float(win), float(place), float(show))
            nk[slug] = nk12
    return preds, nk


def reliability(samples, bins, label):
    """samples = [(pred, hit_bool)] -> 確率帯ごとの (n, mean_pred, emp) と ECE を出力。"""
    n_total = len(samples)
    print(f"\n=== {label} reliability（{n_total} 頭・帯ごと 予測 vs 実測）===")
    print(f"{'帯':>12} {'n':>5} {'平均予測':>8} {'実測':>8} {'差':>7}")
    ece = 0.0
    for i in range(len(bins) - 1):
        lo, hi = bins[i], bins[i + 1]
        b = [(p, h) for p, h in samples if lo <= p < hi]
        if not b:
            continue
        mp = statistics.mean(p for p, _ in b)
        emp = sum(1 for _, h in b if h) / len(b)
        ece += len(b) / n_total * abs(emp - mp)
        print(f"{lo * 100:>4.0f}-{hi * 100:>3.0f}% {len(b):>5} {mp * 100:>7.1f}% {emp * 100:>7.1f}% {(emp - mp) * 100:>+6.1f}")
    print(f"ECE = {ece * 100:.2f}%")
    return ece


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pure", default="/tmp/bt252/pure_preds.tsv")
    ap.add_argument("--results-dir", default="/tmp/bt252")
    ap.add_argument("--winodds", default="/tmp/bt252/bt_winodds.tsv")
    args = ap.parse_args()

    preds, nk = load_pure(args.pure)
    winodds = ub.parse_winodds(args.winodds)

    widths = []
    model_top1 = market_top1 = races_scored = market_races = 0
    win_samples, place_samples, show_samples = [], [], []  # 純モデル
    mkt_win_samples = []                                    # 市場 implied（対照）
    longshot = []                                           # (pred_win, won) for モデル高×市場薄

    for slug, horses in preds.items():
        resf = Path(args.results_dir) / f"res_{nk[slug]}.html"
        if not resf.exists():
            continue
        top3, _ = ub.parse_result(resf)
        if len(top3) < 3:
            continue
        winner = top3[0]
        place_set, show_set = set(top3[:2]), set(top3[:3])

        wins = {num: w for num, (w, _, _) in horses.items()}
        widths.append(max(wins.values()) - min(wins.values()))

        # top-1（モデル）。winner が pred に居るレースのみ採点。
        if winner in wins:
            races_scored += 1
            if max(wins, key=wins.get) == winner:
                model_top1 += 1

        # 市場 implied（対照）: odds→implied 正規化、本命=人気1位。
        mkt = winodds.get(slug, {})
        if mkt:
            inv = {num: 1.0 / o for num, (_, o) in mkt.items() if o > 0}
            s = sum(inv.values())
            if s > 0 and winner in inv:
                market_races += 1
                fav = min(mkt, key=lambda n: mkt[n][0])  # 人気1位
                if fav == winner:
                    market_top1 += 1
                for num, iv in inv.items():
                    mkt_win_samples.append((iv / s, num == winner))

        for num, (w, pl, sh) in horses.items():
            win_samples.append((w, num == winner))
            place_samples.append((pl, num in place_set))
            show_samples.append((sh, num in show_set))
            o = mkt.get(num, (None, None))[1]
            if w >= 0.05 and o is not None and o >= 20.0:  # モデル高 win × 市場薄（20倍以上）
                longshot.append((w, num == winner))

    if not widths:
        sys.exit("対象レースなし: --pure / --results-dir / --winodds のパスとデータを確認")

    # --- モデル幅（フラットさ）---
    widths.sort()
    print(f"対象 {len(widths)}R / 採点 {races_scored}R\n")
    print("=== モデル幅 win(最大−最小) の分布 ＝ フラットさ ===")
    print(f"  中央値 {statistics.median(widths) * 100:.1f}pt / 最小 {widths[0] * 100:.1f}pt / "
          f"最大 {widths[-1] * 100:.1f}pt / 平均 {statistics.mean(widths) * 100:.1f}pt")
    flat = sum(1 for w in widths if w < 0.08)
    print(f"  幅 < 8pt（フラット）: {flat}/{len(widths)}R")

    # --- top-1 的中 ---
    # 注: 純モデルの分母 races_scored（winner が pred 表に居るレース）と市場本命の分母
    # market_races（winner が odds に居るレース）は厳密には別集合。両件数を併記して透明にする。
    # また win は gen_pure_preds が表示テーブルから読む 0.1% 量子化値なので、最上位が同率だと
    # argmax は dict 順で先勝ちする（極稀。相対比較が用途なので許容）。
    print("\n=== top-1 的中（argmax が勝つか）===")
    if races_scored:
        print(f"  純モデル: {model_top1}/{races_scored} = {model_top1 / races_scored * 100:.1f}%")
    if market_races:
        print(f"  市場本命: {market_top1}/{market_races} = {market_top1 / market_races * 100:.1f}%（対照）")
    else:
        # 市場対照が 1 件も採れない＝winodds の id と pure_preds の slug が不一致の可能性。
        # クラッシュせず「対照だけ消える」事故になるので気づけるよう警告する。
        print("WARN: 市場対照が空（--winodds の id と --pure の slug が不一致の可能性）", file=sys.stderr)

    # --- reliability / ECE ---
    # 注: win 純モデルは全レース全頭、市場 implied は winner∈odds のレースのみ採録で母集団が
    # 厳密には別（勝者は常に odds>0 を持つため実質バイアスは無視できる範囲）。
    reliability(win_samples, WIN_BINS, "win 純モデル")
    if mkt_win_samples:
        reliability(mkt_win_samples, WIN_BINS, "win 市場 implied（対照）")
    reliability(place_samples, PLACE_BINS, "place 純モデル")
    reliability(show_samples, SHOW_BINS, "show 純モデル")

    # --- 大穴過大評価 ---
    print("\n=== 大穴過大評価（モデル win≥5% かつ 市場20倍以上）===")
    if longshot:
        mp = statistics.mean(p for p, _ in longshot)
        emp = sum(1 for _, h in longshot if h) / len(longshot)
        print(f"  {len(longshot)} 頭: 平均予測 {mp * 100:.1f}% / 実勝率 {emp * 100:.1f}% "
              f"→ 過大評価 {(mp - emp) * 100:+.1f}pt")
    else:
        print("  該当なし")


if __name__ == "__main__":
    main()
