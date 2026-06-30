#!/usr/bin/env python3
"""train_gbm.py（非線形 GBM ハーネス）の基本ロジックの単体テスト（numpy/sklearn 必須）。

実行: scripts/harness/.venv/bin/python -m unittest test_train_gbm
"""

import unittest

import numpy as np

import train_gbm as G
import train_pl as T


_RNG = np.random.default_rng(42)


def _race(rid, date, feat_col0, winner, odds):
    """先頭列で勝敗が決まる最小レース。他列は HGB が binning できるよう非退化なノイズで埋める
    （全 NaN 列だと sklearn の binning が落ちるため。実データは非退化）。"""
    n = len(feat_col0)
    feat = _RNG.normal(size=(n, len(G.GBM_FEATURES)))
    feat[:, 0] = feat_col0
    return T.Race(
        race_id=rid,
        date=date,
        horse_nums=list(range(1, n + 1)),
        fund=feat,
        winner=winner,
        win_odds=np.array(odds, dtype=float),
        baseline=np.full(n, 1.0 / n),
    )


class HelpersTest(unittest.TestCase):
    def test_gbm_features_include_rates_and_starts(self):
        # 9 基礎（rate6+signal3）＋ starts6 = 15。
        self.assertEqual(len(G.GBM_FEATURES), 15)
        self.assertTrue(all(s in G.GBM_FEATURES for s in G.STARTS_FEATURES))

    def test_normalize_within_race_sums_to_one(self):
        p = G._normalize_within_race(np.array([0.2, 0.6, 0.2]))
        self.assertAlmostEqual(float(p.sum()), 1.0)
        np.testing.assert_allclose(p, [0.2, 0.6, 0.2])

    def test_normalize_zero_falls_back_to_uniform(self):
        p = G._normalize_within_race(np.array([0.0, 0.0]))
        np.testing.assert_allclose(p, [0.5, 0.5])

    def test_design_appends_market_column(self):
        r = _race("R", "2025-01-01", [0.1, 0.2, 0.3], 0, [2.0, 4.0, 4.0])
        self.assertEqual(G._design(r, use_market=False).shape, (3, 15))
        self.assertEqual(G._design(r, use_market=True).shape, (3, 16))


class WalkForwardSmokeTest(unittest.TestCase):
    def test_walk_forward_returns_normalized_oos_only(self):
        rng = np.random.default_rng(0)
        races = []
        for k in range(80):
            x0 = rng.normal(size=4)
            w = int(np.argmax(x0))
            month = "2025-01" if k < 50 else "2025-02"
            races.append(_race(f"R{k}", f"{month}-15", x0, w, [3.0, 3.0, 3.0, 3.0]))
        cutoffs = T.monthly_cutoffs(races, "2025-02-01")
        params = dict(max_iter=20, learning_rate=0.1, max_leaf_nodes=7, min_samples_leaf=5, early_stopping=False, random_state=0)
        preds = G.walk_forward(races, cutoffs, use_market=False, params=params)
        # OOS は 2 月の 30 レースのみ、各予測はレース内で正規化されている。
        oos_ids = {r.race_id for r in races if r.date >= "2025-02-01"}
        self.assertEqual(set(preds), oos_ids)
        for p in preds.values():
            self.assertAlmostEqual(float(p.sum()), 1.0, places=6)


if __name__ == "__main__":
    unittest.main()
