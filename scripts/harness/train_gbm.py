#!/usr/bin/env python3
"""非線形 GBM 木（#309 Phase B）: ヒストグラム勾配ブースティングで win を学習し、線形 PL で
崩壊した fundamental が木の非線形・交互作用で marginal シグナルを出すかを walk-forward で検証する。

LightGBM は libomp（OpenMP の共有ライブラリ）を要求し未導入環境でロードできないため、libomp 不要で
NaN ネイティブ対応の sklearn `HistGradientBoostingClassifier`（同じヒストグラム勾配ブースティング系）を
使う。評価軸・walk-forward・OOS 集合は `train_pl.py` と共通（同じ as-of ダンプ・日付分割でリーク無し）。

特徴量は PL の 9（factor 勝率6＋signal3）に加え、信頼度として factor の出走数 starts 6 を足した 15
（木は rate×starts 交互作用を使える）。欠落は補完せず NaN のまま渡す（HGB が分岐で扱う）。市場版は
レース内正規化した単勝 implied を 1 列追加する。予測はレース内で正規化して win 確率分布にする。

使い方: scripts/harness/.venv/bin/python scripts/harness/train_gbm.py scripts/harness/data/dump_full.tsv
"""

import argparse
import sys

import numpy as np
from sklearn.ensemble import HistGradientBoostingClassifier

import train_pl as T

# starts（factor の出走数＝信頼度）。rate と対で木に渡す。
STARTS_FEATURES = [
    "course_gate_starts",
    "horse_surface_starts",
    "horse_distance_starts",
    "jockey_surface_starts",
    "trainer_surface_starts",
    "horse_track_condition_starts",
]
GBM_FEATURES = T.FUND_FEATURES + STARTS_FEATURES


def load_gbm(path):
    """ダンプを [`train_pl.Race`] のリストに読む。`fund` は GBM 特徴量行列（15 列・NaN 保持）。

    `train_pl.load_races` に列セットを渡して使い回す（`train_pl.evaluate` は `fund` を参照せず
    winner/win_odds/baseline のみ使うので、`fund` に GBM の特徴量行列が入っていてよい）。"""
    return T.load_races(path, GBM_FEATURES)


def _design(race, use_market):
    if not use_market:
        return race.fund
    imp = T.market_implied(race.win_odds).reshape(-1, 1)
    return np.hstack([race.fund, imp])


def _normalize_within_race(p):
    s = p.sum()
    return p / s if s > 0 else np.full(len(p), 1.0 / len(p))


def walk_forward(races, cutoffs, use_market, params):
    """expanding window で HGB を学習し OOS の win 確率（レース内正規化）を返す。"""
    preds = {}
    for k, cut in enumerate(cutoffs):
        nxt = cutoffs[k + 1] if k + 1 < len(cutoffs) else "9999-99-99"
        train = [r for r in races if r.date < cut and r.winner is not None]
        test = [r for r in races if cut <= r.date < nxt]
        if not train or not test:
            continue
        x_tr = np.vstack([_design(r, use_market) for r in train])
        # train は winner ありに限定済みなので one-hot は常に作れる（winner 行=1・他=0）。
        y_tr = np.concatenate([np.eye(len(r.horse_nums))[r.winner] for r in train])
        clf = HistGradientBoostingClassifier(**params)
        clf.fit(x_tr, y_tr)
        for race in test:
            proba = clf.predict_proba(_design(race, use_market))[:, 1]
            preds[race.race_id] = _normalize_within_race(proba)
    return preds


def main(argv=None):
    ap = argparse.ArgumentParser(description="HGB(非線形GBM) の walk-forward 評価（#309）")
    ap.add_argument("dump", help="analyze backtest --dump-features の出力 TSV（全期間）")
    ap.add_argument("--oos-start", default="2025-07", help="OOS 開始月 YYYY-MM（既定 2025-07）")
    ap.add_argument("--max-iter", type=int, default=150)
    ap.add_argument("--learning-rate", type=float, default=0.05)
    ap.add_argument("--max-leaf-nodes", type=int, default=15)
    ap.add_argument("--min-samples-leaf", type=int, default=100)
    ap.add_argument("--l2", type=float, default=1.0, help="HGB の l2_regularization")
    args = ap.parse_args(argv)

    params = dict(
        max_iter=args.max_iter,
        learning_rate=args.learning_rate,
        max_leaf_nodes=args.max_leaf_nodes,
        min_samples_leaf=args.min_samples_leaf,
        l2_regularization=args.l2,
        early_stopping=False,
        random_state=0,
    )

    races = load_gbm(args.dump)
    cutoffs = T.monthly_cutoffs(races, args.oos_start + "-01")
    if not cutoffs:
        print("OOS 窓が空（データ期間と --oos-start を確認）", file=sys.stderr)
        return 1

    preds_fund = walk_forward(races, cutoffs, use_market=False, params=params)
    preds_mkt = walk_forward(races, cutoffs, use_market=True, params=params)

    oos = [r for r in races if r.race_id in preds_fund]
    n1, m_fund = T.evaluate(oos, preds_fund)
    _, m_mkt = T.evaluate(oos, preds_mkt)

    print(
        f"# HGB(非線形GBM) walk-forward 評価（OOS {args.oos_start}〜 / {n1} レース / "
        f"max_iter={args.max_iter} leaves={args.max_leaf_nodes} min_leaf={args.min_samples_leaf}）"
    )
    print(f"{'モデル':<22} {'Brier':>9} {'LogLoss':>9} {'flat 払戻率':>9}")

    def line(label, m):
        print(f"{label:<22} {m['brier']:>9.4f} {m['logloss']:>9.4f} {m['roi'] * 100:>8.1f}%")

    line("HGB 基礎(fund)", m_fund["pl"])
    line("HGB 市場あり(fund+mkt)", m_mkt["pl"])
    line("baseline(α=0.2)", m_fund["baseline"])
    line("純市場(implied)", m_fund["market"])
    print(
        "\n注: 「flat 払戻率」はトップ選好馬の単勝 100 円固定の総払戻倍率／賭けレース数（net ROI でなく"
        "粗の払戻率）。Brier/LogLoss は per-horse スコア（全モデル共通母数）。木の特徴は rate6+signal3+"
        "starts6（市場版は +implied）。採否は複数指標で判断する。"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
