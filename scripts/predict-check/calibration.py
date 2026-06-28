#!/usr/bin/env python3
"""純モデル（α=0）の校正・フラットさ測定（#286 のベースライン）。

gen_pure_preds.py が出した純モデル prob と netkeiba 実結果を突き合わせ、以下を出す:
  - モデル幅（各レースの win 最大−最小）の分布＝フラットさの定量化
  - top-1 的中（argmax win が勝つか）。市場本命（人気1位）をベースラインに併記
  - win / show の reliability（確率帯ごとの予測 vs 実測）と ECE。市場 implied も併記
  - 大穴過大評価（モデル高 win × 市場低人気の馬の実勝率）

入力は umaren_backtest の parse_result / parse_winodds を再利用。

使い方:
    python3 scripts/predict-check/calibration.py \
        --pure /tmp/bt252/pure_preds.tsv --results-dir /tmp/bt252 \
        --winodds /tmp/bt252/bt_winodds.tsv
"""
import argparse
import math
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import umaren_backtest as ub  # noqa: E402

WIN_BINS = [0.0, 0.02, 0.05, 0.10, 0.20, 0.40, 1.01]
SHOW_BINS = [0.0, 0.10, 0.20, 0.30, 0.45, 0.60, 1.01]
PLACE_BINS = [0.0, 0.05, 0.12, 0.20, 0.30, 0.45, 1.01]


def load_pure(path):
    """pure_preds.tsv -> ({slug: {num: (win,place,show)}}, {slug: nk12})."""
    preds, nk = {}, {}
    with open(path) as f:
        next(f)  # header
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


def ece_of(samples, bins):
    """reliability の ECE だけを静かに返す（掃引用）。"""
    n = len(samples)
    if n == 0:
        return 0.0
    e = 0.0
    for i in range(len(bins) - 1):
        lo, hi = bins[i], bins[i + 1]
        b = [(p, h) for p, h in samples if lo <= p < hi]
        if not b:
            continue
        mp = statistics.mean(p for p, _ in b)
        emp = sum(1 for _, h in b if h) / len(b)
        e += len(b) / n * abs(emp - mp)
    return e


def power_renorm(vals, target, gamma):
    """vals を ^gamma して合計 target に再正規化（各要素 ≤1.0）。rust apply_placeshow_power と同型。

    合計が 0/非有限のときは None（rust の no-op 判定に合わせる）。
    """
    powered = [v ** gamma for v in vals]
    total = sum(powered)
    if total <= 0 or not math.isfinite(total):
        return None
    return [min(1.0, p / total * target) for p in powered]


def placeshow_after_power(horses, gamma):
    """1 レースの {num:(win,place,show)} に place/show 冪 gamma を後掛け（win 不変・単調再是正）。

    place^gamma→合計2.0、show^gamma→合計3.0 に再正規化し、win ≤ place ≤ show を累積 max で是正。
    place・show のいずれかが再正規化不能（合計0/非有限）なら rust 同様レース全体を no-op（入力そのまま）。
    """
    nums = list(horses)
    wins = [horses[n][0] for n in nums]
    place = power_renorm([horses[n][1] for n in nums], 2.0, gamma)
    show = power_renorm([horses[n][2] for n in nums], 3.0, gamma)
    if place is None or show is None:
        return dict(horses)
    out = {}
    for i, n in enumerate(nums):
        pl = min(1.0, max(place[i], wins[i]))
        sh = min(1.0, max(show[i], pl))
        out[n] = (wins[i], pl, sh)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pure", default="/tmp/bt252/pure_preds.tsv")
    ap.add_argument("--results-dir", default="/tmp/bt252")
    ap.add_argument("--winodds", default="/tmp/bt252/bt_winodds.tsv")
    ap.add_argument("--placeshow-power-grid", default="1.0,1.25,1.5,1.75,2.0,2.5",
                    help="place/show 冪 γ の掃引（show/place ECE 最小を選ぶ）")
    args = ap.parse_args()

    preds, nk = load_pure(args.pure)
    winodds = ub.parse_winodds(args.winodds)

    widths = []
    model_top1 = market_top1 = races_scored = market_races = 0
    win_samples, show_samples = [], []          # 純モデル
    mkt_win_samples = []                         # 市場 implied（対照）
    longshot = []                               # (pred_win, won) for モデル高×市場薄
    scored = []                                 # 掃引用 per-race: (horses, winner, place_set, show_set)

    for slug, horses in preds.items():
        resf = Path(args.results_dir) / f"res_{nk[slug]}.html"
        if not resf.exists():
            continue
        top3, _ = ub.parse_result(resf)
        if len(top3) < 3:
            continue
        winner, show_set = top3[0], set(top3[:3])
        scored.append((horses, winner, set(top3[:2]), show_set))

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

        for num, (w, _, sh) in horses.items():
            win_samples.append((w, num == winner))
            show_samples.append((sh, num in show_set))
            o = mkt.get(num, (None, None))[1]
            if w >= 0.05 and o is not None and o >= 20.0:  # モデル高 win × 市場薄（20倍以上）
                longshot.append((w, num == winner))

    # --- モデル幅（フラットさ）---
    widths.sort()
    print(f"対象 {len(widths)}R / 採点 {races_scored}R\n")
    print("=== モデル幅 win(最大−最小) の分布 ＝ フラットさ ===")
    print(f"  中央値 {statistics.median(widths) * 100:.1f}pt / 最小 {widths[0] * 100:.1f}pt / "
          f"最大 {widths[-1] * 100:.1f}pt / 平均 {statistics.mean(widths) * 100:.1f}pt")
    flat = sum(1 for w in widths if w < 0.08)
    print(f"  幅 < 8pt（フラット）: {flat}/{len(widths)}R")

    # --- top-1 的中 ---
    print("\n=== top-1 的中（argmax が勝つか）===")
    if races_scored:
        print(f"  純モデル: {model_top1}/{races_scored} = {model_top1 / races_scored * 100:.1f}%")
    if market_races:
        print(f"  市場本命: {market_top1}/{market_races} = {market_top1 / market_races * 100:.1f}%（対照）")

    # --- reliability / ECE ---
    reliability(win_samples, WIN_BINS, "win 純モデル")
    if mkt_win_samples:
        reliability(mkt_win_samples, WIN_BINS, "win 市場 implied（対照）")
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

    # --- place/show 冪 γ の掃引（show/place ECE 最小を選ぶ。win 不変）---
    # 注: γ=1.0 行は厳密な現行 no-op ではなく、place→合計2.0/show→合計3.0 へ再正規化＋単調再是正を
    # 通した baseline（rust の placeshow_power=None とは別）。実測 reliability の show ECE とほぼ一致する。
    grid = [float(x) for x in args.placeshow_power_grid.split(",")]
    print("\n=== place/show 冪 γ 掃引（ECE 最小を採用。win は不変）===")
    print(f"{'γ':>5} {'show ECE':>9} {'place ECE':>10}")
    for g in grid:
        ss, ps = [], []
        for horses, winner, place_set, show_set in scored:
            t = placeshow_after_power(horses, g)
            for num, (_, pl, sh) in t.items():
                ps.append((pl, num in place_set))
                ss.append((sh, num in show_set))
        print(f"{g:>5.2f} {ece_of(ss, SHOW_BINS) * 100:>8.2f}% {ece_of(ps, PLACE_BINS) * 100:>9.2f}%")


if __name__ == "__main__":
    main()
