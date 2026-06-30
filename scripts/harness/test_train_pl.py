#!/usr/bin/env python3
"""train_pl.py の学習・予測・評価ロジックの単体テスト（numpy/scipy 必須）。

合成データで「条件付きロジットが正しい符号の係数を学習する」「softmax 予測が正規化される」
「walk-forward がリーク無しで OOS 予測を返す」「メトリクスが手計算と一致」を固定する。

実行: scripts/harness/.venv/bin/python -m unittest test_train_pl
"""

import math
import unittest

import numpy as np

import train_pl as T


def _race(rid, date, fund_col0, winner, odds):
    """1 特徴量だけ動かす最小レース（他 fund 列は中立 0.5、baseline は一様）。"""
    n = len(fund_col0)
    fund = np.full((n, len(T.FUND_FEATURES)), 0.5)
    fund[:, 0] = fund_col0
    return T.Race(
        race_id=rid,
        date=date,
        horse_nums=list(range(1, n + 1)),
        fund=fund,
        winner=winner,
        win_odds=np.array(odds, dtype=float),
        baseline=np.full(n, 1.0 / n),
    )


class ConditionalLogitTest(unittest.TestCase):
    def _separable_races(self, n=200):
        # 各レース 4 頭、特徴量 0 が最大の馬が必ず勝つ → β[0] は正に学習されるはず。
        rng = np.random.default_rng(0)
        races = []
        for k in range(n):
            x0 = rng.normal(size=4)
            winner = int(np.argmax(x0))
            races.append(_race(f"R{k}", "2025-01-01", x0, winner, [3.0, 3.0, 3.0, 3.0]))
        return races

    def test_fit_recovers_positive_weight(self):
        races = self._separable_races()
        impute, mean, std = T.fit_stats(races)
        designs = T.build_design(races, use_market=False, impute=impute, mean=mean, std=std)
        beta = T.fit_conditional_logit(designs, [r.winner for r in races], l2=1.0)
        # 特徴量 0 の係数が支配的に正（勝者を当てる方向）。
        self.assertGreater(beta[0], 0.5)
        self.assertEqual(np.argmax(np.abs(beta)), 0)

    def test_predict_is_normalized_and_ranks_top_feature(self):
        races = self._separable_races()
        impute, mean, std = T.fit_stats(races)
        designs = T.build_design(races, use_market=False, impute=impute, mean=mean, std=std)
        beta = T.fit_conditional_logit(designs, [r.winner for r in races], l2=1.0)
        preds = T.predict(designs, beta)
        for race, p in zip(races, preds):
            self.assertAlmostEqual(float(p.sum()), 1.0, places=9)
            # 特徴量 0 最大の馬が最大確率。
            self.assertEqual(int(np.argmax(p)), race.winner)

    def test_l2_shrinks_coefficients(self):
        races = self._separable_races()
        impute, mean, std = T.fit_stats(races)
        designs = T.build_design(races, use_market=False, impute=impute, mean=mean, std=std)
        weak = T.fit_conditional_logit(designs, [r.winner for r in races], l2=1.0)
        strong = T.fit_conditional_logit(designs, [r.winner for r in races], l2=1000.0)
        self.assertLess(np.linalg.norm(strong), np.linalg.norm(weak))


class MarketAndImputeTest(unittest.TestCase):
    def test_market_implied_normalizes(self):
        imp = T.market_implied(np.array([2.0, 4.0, 4.0]))
        self.assertAlmostEqual(float(imp.sum()), 1.0)
        # オッズが低い（=人気）ほど implied は高い。
        self.assertGreater(imp[0], imp[1])

    def test_market_implied_handles_missing(self):
        imp = T.market_implied(np.array([math.nan, math.nan]))
        np.testing.assert_allclose(imp, [0.5, 0.5])

    def test_fit_stats_imputes_signals_with_neutral(self):
        # signal 列（index 6=recent_form）が全欠落でも中立 0.5 で補完される。
        r = _race("R", "2025-01-01", [0.1, 0.2], 0, [3.0, 3.0])
        r.fund[:, 6] = math.nan
        impute, _, _ = T.fit_stats([r])
        self.assertAlmostEqual(impute[6], 0.5)


class WalkForwardTest(unittest.TestCase):
    def test_walk_forward_predicts_only_future_windows(self):
        # 2 か月分。OOS は 2 月のみ（1 月は訓練）。1 月レースは preds に含まれない。
        rng = np.random.default_rng(1)
        races = []
        for k in range(60):
            x0 = rng.normal(size=4)
            w = int(np.argmax(x0))
            month = "2025-01" if k < 40 else "2025-02"
            races.append(_race(f"R{k}", f"{month}-15", x0, w, [3.0, 3.0, 3.0, 3.0]))
        cutoffs = T.monthly_cutoffs(races, "2025-02-01")
        preds = T.walk_forward(races, cutoffs, use_market=False, l2=1.0)
        self.assertTrue(all(rid.startswith("R") for rid in preds))
        # OOS は 2 月の 20 レースのみ。
        self.assertEqual(len(preds), 20)
        oos_ids = {r.race_id for r in races if r.date >= "2025-02-01"}
        self.assertEqual(set(preds), oos_ids)


class EvaluateTest(unittest.TestCase):
    def test_accumulator_brier_and_roi(self):
        acc = T._Acc()
        # 2 頭・勝者 index0・予測 [0.6,0.4]・オッズ [2.0,5.0]。top=0=勝者 → 払戻 2.0。
        acc.add_race(np.array([0.6, 0.4]), np.array([1.0, 0.0]), winner=0, win_odds=np.array([2.0, 5.0]))
        s = acc.summary()
        self.assertAlmostEqual(s["brier"], ((0.6 - 1) ** 2 + (0.4 - 0) ** 2) / 2)
        self.assertEqual(s["payout_races"], 1)
        self.assertAlmostEqual(s["roi"], 2.0)

    def test_accumulator_miss_yields_zero_payout(self):
        acc = T._Acc()
        # top=0 だが勝者は index1 → 払戻 0、母数 1。
        acc.add_race(np.array([0.7, 0.3]), np.array([0.0, 1.0]), winner=1, win_odds=np.array([2.0, 5.0]))
        s = acc.summary()
        self.assertEqual(s["payout_races"], 1)
        self.assertAlmostEqual(s["roi"], 0.0)


if __name__ == "__main__":
    unittest.main()
