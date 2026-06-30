#!/usr/bin/env python3
"""学習型 fundamental モデル（#309 Phase B）: レース内 softmax の条件付きロジット（=
Plackett-Luce の win 段）を walk-forward で訓練し、α=0.2 baseline・純市場と out-of-sample で
比較する。リーク防止は `analyze backtest --dump-features` の as-of ダンプ（特徴量は予測対象日
`< D` の統計）＋日付分割（訓練は予測窓より前の日付のみ）で担保する。

依存: numpy / scipy（requirements.txt）。忠実性サニティ③（faithfulness.py）は stdlib のみ。

モデル: レース r・馬 i について P(i が 1 着) = softmax_i(β·x_i)。winner（finishing_position==1）の
条件付き対数尤度を L2 正則化付きで最大化（McFadden 条件付きロジット）。softmax はレベル不変
なので切片は不要。まず win 段のみ（place/show は PL 拡張で follow-up）。

使い方:
  scripts/harness/.venv/bin/python scripts/harness/train_pl.py scripts/harness/data/dump_full.tsv
"""

import argparse
import csv
import math
import sys

import numpy as np
from scipy.optimize import minimize

# 基礎特徴量（factor 勝率6＋signal3）。dump のヘッダ列名と一致させる。
FUND_FEATURES = [
    "course_gate_win",
    "horse_surface_win",
    "horse_distance_win",
    "jockey_surface_win",
    "trainer_surface_win",
    "horse_track_condition_win",
    "recent_form",
    "weight_carried",
    "jockey_recent_form",
]
# signal は 0.5 が中立。欠落はこの値で補完する（factor 勝率は訓練 fold 平均で補完）。
SIGNAL_NEUTRAL = {"recent_form": 0.5, "weight_carried": 0.5, "jockey_recent_form": 0.5}
LOG_LOSS_EPS = 1e-15


class Race:
    """1 レース分の発走馬の特徴量・ラベル・市場・baseline をまとめた最小コンテナ。"""

    __slots__ = ("race_id", "date", "horse_nums", "fund", "winner", "win_odds", "baseline")

    def __init__(self, race_id, date, horse_nums, fund, winner, win_odds, baseline):
        self.race_id = race_id
        self.date = date
        self.horse_nums = horse_nums  # list[int]
        self.fund = fund  # np.ndarray (n_horses, n_fund) 生値（欠落は NaN）
        self.winner = winner  # int index（1 着の行）or None
        self.win_odds = win_odds  # np.ndarray (n_horses,) 単勝オッズ（欠落 NaN）
        self.baseline = baseline  # np.ndarray (n_horses,) α=0.2 baseline の win 確率


def _f(s):
    return float(s) if s not in ("", None) else math.nan


def load_races(path, feature_cols=FUND_FEATURES):
    """ダンプ TSV をレース単位の [`Race`] リストに読む（日付昇順）。`fund` には `feature_cols` の列を
    その順で詰める（PL は 9 基礎、GBM は基礎＋starts の 15 を渡す）。`Race.fund` 以外（winner/win_odds/
    baseline）は列セットに依らず共通。`finishing_position` はラベルにのみ使い特徴量には混入しない。"""
    by_race = {}
    order = []
    with open(path, newline="", encoding="utf-8") as fh:
        for r in csv.DictReader(fh, delimiter="\t"):
            rid = r["race_id"]
            if rid not in by_race:
                by_race[rid] = []
                order.append(rid)
            by_race[rid].append(r)
    races = []
    for rid in order:
        rows = by_race[rid]
        n = len(rows)
        fund = np.full((n, len(feature_cols)), math.nan)
        win_odds = np.full(n, math.nan)
        baseline = np.full(n, math.nan)
        horse_nums = []
        winner = None
        for i, row in enumerate(rows):
            horse_nums.append(int(row["horse_num"]))
            for j, col in enumerate(feature_cols):
                fund[i, j] = _f(row[col])
            win_odds[i] = _f(row["win_odds"])
            baseline[i] = _f(row["model_win"])
            fp = row["finishing_position"]
            # 1 着＝winner。同着 1 着（稀）は先頭の 1 頭のみ winner とし他は loser 扱い（全モデル共通で
            # 比較は公平・影響は軽微）。winner なしレースは訓練から除外、評価では全 0 ラベル（母数に残る）。
            if fp not in ("", None) and int(fp) == 1 and winner is None:
                winner = i
        races.append(
            Race(rid, rows[0]["date"], horse_nums, fund, winner, win_odds, baseline)
        )
    return races


def market_implied(win_odds):
    """単勝オッズ→レース内正規化した implied 勝率（控除率込みの素朴版）。欠落は一様で補完。"""
    # 有効オッズのみ逆数を取る。`where=` でマスク要素は除算自体を行わず（0 割の RuntimeWarning 回避）、
    # `out` の初期値 NaN がそのまま残る。
    mask = np.isfinite(win_odds) & (win_odds > 0)
    inv = np.full(len(win_odds), math.nan)
    np.divide(1.0, win_odds, out=inv, where=mask)
    if np.all(np.isnan(inv)):
        return np.full(len(win_odds), 1.0 / len(win_odds))
    # 欠落馬はレース内の有効平均で補完してから正規化。
    mean_inv = np.nanmean(inv)
    inv = np.where(np.isnan(inv), mean_inv, inv)
    return inv / inv.sum()


def build_design(races, use_market, impute, mean, std):
    """各レースの特徴量行列 X（標準化済み）を作る。`impute`/`mean`/`std` は訓練 fold 由来。

    返り値: list[np.ndarray]（レースごと (n_horses, n_features)）。market 版は log(implied) 列を付す。
    """
    designs = []
    for race in races:
        x = race.fund.copy()
        # factor/signal の欠落補完（列ごとの impute 値）。
        for j in range(x.shape[1]):
            col = x[:, j]
            col[np.isnan(col)] = impute[j]
        xs = (x - mean) / std
        if use_market:
            imp = market_implied(race.win_odds)
            logimp = np.log(np.clip(imp, 1e-12, None)).reshape(-1, 1)
            xs = np.hstack([xs, logimp])
        designs.append(xs)
    return designs


def fit_stats(train_races):
    """訓練 fold の factor/signal 補完値（列平均）と標準化統計（平均・標準偏差）を返す。

    補完・signal 中立値は PL の基礎特徴量レイアウト（[`FUND_FEATURES`]）前提。`Race.fund` の列が
    これと一致することを assert で固定する（`load_races(feature_cols=...)` に別の列セットを渡して PL
    経路に流すと無言で列対応がズレるのを防ぐ。GBM 経路は fit_stats を呼ばず NaN ネイティブ）。"""
    stacked = np.vstack([r.fund for r in train_races])
    assert stacked.shape[1] == len(FUND_FEATURES), (
        "fit_stats は FUND_FEATURES レイアウト前提（PL 基礎特徴量）"
    )

    def col_impute(j, col):
        if col in SIGNAL_NEUTRAL:
            return SIGNAL_NEUTRAL[col]
        vals = stacked[:, j]
        vals = vals[np.isfinite(vals)]
        # 当該 factor が訓練 fold で全欠落なら中立 0.0（標準化後 0）にフォールバック。
        return float(vals.mean()) if vals.size else 0.0

    impute = np.array([col_impute(j, col) for j, col in enumerate(FUND_FEATURES)])
    # 補完してから標準化統計を取る。
    filled = stacked.copy()
    for j in range(filled.shape[1]):
        c = filled[:, j]
        c[np.isnan(c)] = impute[j]
    mean = filled.mean(axis=0)
    std = filled.std(axis=0)
    std[std < 1e-8] = 1.0
    return impute, mean, std


def _neg_log_lik(beta, designs, winners, l2):
    """条件付きロジットの負の対数尤度と勾配（winner ありレースのみ）。"""
    nll = 0.0
    grad = np.zeros_like(beta)
    for x, w in zip(designs, winners):
        if w is None:
            continue
        u = x @ beta
        u -= u.max()  # 数値安定化
        ex = np.exp(u)
        p = ex / ex.sum()
        nll -= math.log(max(p[w], 1e-300))
        grad -= x[w] - p @ x
    nll += 0.5 * l2 * np.dot(beta, beta)
    grad += l2 * beta
    return nll, grad


def fit_conditional_logit(designs, winners, l2):
    """L-BFGS で条件付きロジットを当てはめ、係数 β を返す。"""
    n_feat = designs[0].shape[1]
    res = minimize(
        _neg_log_lik,
        np.zeros(n_feat),
        args=(designs, winners, l2),
        jac=True,
        method="L-BFGS-B",
    )
    return res.x


def predict(designs, beta):
    """各レースの softmax(β·x) を返す（レースごと np.ndarray）。"""
    out = []
    for x in designs:
        u = x @ beta
        u -= u.max()
        ex = np.exp(u)
        out.append(ex / ex.sum())
    return out


def walk_forward(races, cutoffs, use_market, l2):
    """expanding window で OOS 予測を返す: dict race_id -> np.ndarray(win prob)。

    各 cutoff D について、訓練=日付 < D、予測=[D, 次の cutoff)。標準化・補完は訓練 fold で確定。
    """
    preds = {}
    for k, cut in enumerate(cutoffs):
        nxt = cutoffs[k + 1] if k + 1 < len(cutoffs) else "9999-99-99"
        train = [r for r in races if r.date < cut and r.winner is not None]
        test = [r for r in races if cut <= r.date < nxt]
        if not train or not test:
            continue
        impute, mean, std = fit_stats(train)
        tr_designs = build_design(train, use_market, impute, mean, std)
        beta = fit_conditional_logit(tr_designs, [r.winner for r in train], l2)
        te_designs = build_design(test, use_market, impute, mean, std)
        for race, p in zip(test, predict(te_designs, beta)):
            preds[race.race_id] = p
    return preds


def monthly_cutoffs(races, start):
    """`start`(YYYY-MM-01) 以降の月初を OOS 窓の cutoff 列にする（dump の date は YYYY-MM-DD 文字列）。"""
    months = sorted({r.date[:7] for r in races if r.date[:7] + "-01" >= start})
    return [f"{m}-01" for m in months]


def evaluate(races, preds):
    """OOS レース集合で Brier(win)/LogLoss(win)/flat ROI を、PL・baseline・市場で算出する。

    母数は OOS の全レース・全出走馬（`analyze backtest` と同じ慣習）。winner 不明レース（着順が全馬
    未確定）も含み、その場合 y は全 0・的中は常に非的中になる。全モデルが同一 OOS 集合・同一母数で
    評価されるため比較は公平。実ダンプは結果確定済みでこの母数バイアスは軽微。"""
    metrics = {"pl": _Acc(), "baseline": _Acc(), "market": _Acc()}
    n_races = 0
    for race in races:
        if race.race_id not in preds:
            continue
        n_races += 1
        y = np.zeros(len(race.horse_nums))
        if race.winner is not None:
            y[race.winner] = 1.0
        series = {
            "pl": preds[race.race_id],
            "baseline": race.baseline,
            "market": market_implied(race.win_odds),
        }
        for name, p in series.items():
            metrics[name].add_race(p, y, race.winner, race.win_odds)
    return n_races, {k: v.summary() for k, v in metrics.items()}


class _Acc:
    """1 モデル分の Brier/LogLoss/flat ROI を蓄積する。"""

    def __init__(self):
        self.brier = 0.0
        self.logloss = 0.0
        self.n_horses = 0
        self.payout = 0.0
        self.payout_races = 0

    def add_race(self, p, y, winner, win_odds):
        p = np.where(np.isfinite(p), p, 0.0)
        self.brier += np.sum((p - y) ** 2)
        pc = np.clip(p, LOG_LOSS_EPS, 1 - LOG_LOSS_EPS)
        self.logloss += float(np.sum(-(y * np.log(pc) + (1 - y) * np.log(1 - pc))))
        self.n_horses += len(p)
        # flat ROI: top 選好馬（同値は馬番昇順＝最初の argmax）の単勝に 1 点。
        top = int(np.argmax(p))
        odds = win_odds[top]
        if np.isfinite(odds):
            self.payout_races += 1
            if winner is not None and top == winner:
                self.payout += float(odds)

    def summary(self):
        return {
            "brier": self.brier / self.n_horses if self.n_horses else float("nan"),
            "logloss": self.logloss / self.n_horses if self.n_horses else float("nan"),
            "roi": self.payout / self.payout_races if self.payout_races else float("nan"),
            "payout_races": self.payout_races,
        }


def main(argv=None):
    ap = argparse.ArgumentParser(description="PL/条件付きロジットの walk-forward 評価（#309）")
    ap.add_argument("dump", help="analyze backtest --dump-features の出力 TSV（全期間）")
    ap.add_argument("--oos-start", default="2025-07", help="OOS 開始月 YYYY-MM（既定 2025-07）")
    ap.add_argument("--l2", type=float, default=10.0, help="L2 正則化強度（既定 10）")
    args = ap.parse_args(argv)

    races = load_races(args.dump)
    cutoffs = monthly_cutoffs(races, args.oos_start + "-01")
    if not cutoffs:
        print("OOS 窓が空（データ期間と --oos-start を確認）", file=sys.stderr)
        return 1

    preds_fund = walk_forward(races, cutoffs, use_market=False, l2=args.l2)
    preds_mkt = walk_forward(races, cutoffs, use_market=True, l2=args.l2)

    oos = [r for r in races if r.race_id in preds_fund]
    n1, m_fund = evaluate(oos, preds_fund)
    _, m_mkt = evaluate(oos, preds_mkt)

    print(f"# PL walk-forward 評価（OOS {args.oos_start}〜 / {n1} レース / L2={args.l2}）")
    print(f"{'モデル':<22} {'Brier':>9} {'LogLoss':>9} {'flat 払戻率':>9}")

    def line(label, m):
        print(
            f"{label:<22} {m['brier']:>9.4f} {m['logloss']:>9.4f} {m['roi'] * 100:>8.1f}%"
        )

    # baseline / market は walk_forward に依らず evaluate 内で算出され、両呼び出しとも同じ OOS 集合な
    # ので同値。fund 側の評価結果から表示する。
    line("PL 基礎(fund)", m_fund["pl"])
    line("PL 市場あり(fund+mkt)", m_mkt["pl"])
    line("baseline(α=0.2)", m_fund["baseline"])
    line("純市場(implied)", m_fund["market"])

    # ADR 0053 の中心証拠を再現可能にするため、全 winner ありレースに当てはめた係数を出す。
    # 「市場を入れると β_market≈1 で市場を再現し fundamental が ±0 へ崩壊」を監査できる。
    # 注: fundamental は標準化済み係数、log_market_implied は log-implied 生値の係数でスケールが
    # 異なる（β_market≈1 は softmax(log implied)=implied＝市場再現を意味する）。L2 は両者を同強度で
    # 罰する（未標準化の市場列にもスケール差のまま適用）が、結果は市場支配・fundamental 崩壊で、
    # 仮に市場列の罰を緩めても fundamental 側へ寄る材料にはならず棄却結論を変えない。
    train_all = [r for r in races if r.winner is not None]
    imp, mean, std = fit_stats(train_all)
    b_fund = fit_conditional_logit(
        build_design(train_all, False, imp, mean, std), [r.winner for r in train_all], args.l2
    )
    b_mkt = fit_conditional_logit(
        build_design(train_all, True, imp, mean, std), [r.winner for r in train_all], args.l2
    )
    print("\n## 学習係数（全 winner ありレース当てはめ・|大|ほど寄与）")
    print("特徴量                       基礎のみ   市場あり")
    for j, name in enumerate(FUND_FEATURES):
        print(f"  {name:<26} {b_fund[j]:+7.3f}  {b_mkt[j]:+7.3f}")
    print(f"  {'log_market_implied':<26} {'—':>7}  {b_mkt[-1]:+7.3f}")

    print(
        "\n注: 「flat 払戻率」は「トップ選好馬の単勝 100 円」固定での総払戻倍率／賭けレース数（net ROI で"
        "なく粗の払戻率）。分母 payout_races は各モデルの top 選好馬のオッズが取れたレースで、モデル間で"
        "わずかに異なりうる（高分散の参考値で採否には使わない）。Brier/LogLoss は全出走馬を独立 Bernoulli"
        "とした per-horse スコア（全モデル共通母数で比較は公平）。控除率 20-25% のため払戻率の 100% 接近は"
        "困難。採否は複数指標で判断する。"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
