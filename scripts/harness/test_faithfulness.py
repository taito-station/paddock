#!/usr/bin/env python3
"""faithfulness.py の集計・突合ロジックの単体テスト（stdlib unittest）。

合成ダンプで手計算と一致することを固定し、ハーネス自体の算術退行を防ぐ。実 DB との
end-to-end 一致は `check_faithfulness.sh`（Rust backtest と突合）が別途担保する。
"""

import math
import unittest

import faithfulness as F


def _row(race_id, horse_num, mw, mp, ms, fp, odds):
    """csv.DictReader 相当（全値 str・欠落は空文字）の 1 行を作る。"""
    return {
        "race_id": race_id,
        "horse_num": str(horse_num),
        "model_win": str(mw),
        "model_place": str(mp),
        "model_show": str(ms),
        "finishing_position": "" if fp is None else str(fp),
        "win_odds": "" if odds is None else str(odds),
    }


# 2 レース×2 頭の合成ダンプ（手計算済み）。
ROWS = [
    _row("R1", 1, 0.6, 0.7, 0.8, 1, 2.0),
    _row("R1", 2, 0.4, 0.5, 0.6, 2, 3.0),
    _row("R2", 1, 0.3, 0.4, 0.5, 2, 5.0),
    _row("R2", 2, 0.7, 0.8, 0.9, 1, 1.5),
]


class ComputeMetricsTest(unittest.TestCase):
    def setUp(self):
        self.m = F.compute_metrics(ROWS)

    def test_race_count_and_hit_rates(self):
        # トップ選好馬: R1=馬番1(0.6), R2=馬番2(0.7)。どちらも 1 着 → 全的中率 100%。
        self.assertEqual(self.m["races"], 2)
        self.assertAlmostEqual(self.m["win_hit"], 1.0)
        self.assertAlmostEqual(self.m["place_hit"], 1.0)
        self.assertAlmostEqual(self.m["show_hit"], 1.0)

    def test_payout_rate(self):
        # R1 トップ 2.0 的中 + R2 トップ 1.5 的中 → (2.0+1.5)/2 = 1.75。
        self.assertEqual(self.m["payout_races"], 2)
        self.assertAlmostEqual(self.m["payout_rate"], 1.75)

    def test_brier(self):
        self.assertAlmostEqual(self.m["brier_win"], 0.125)
        self.assertAlmostEqual(self.m["brier_place"], 0.185)
        self.assertAlmostEqual(self.m["brier_show"], 0.115)

    def test_logloss_win(self):
        expected = (-math.log(0.6) * 2 - math.log(0.7) * 2) / 4
        self.assertAlmostEqual(self.m["logloss_win"], expected)

    def test_top_pick_tie_break_prefers_lower_horse_num(self):
        # 同 model_win なら馬番昇順。馬番2 が 1 着でも、同値のトップは馬番1(着外)。
        rows = [
            _row("T", 1, 0.5, 0.5, 0.5, 3, 4.0),
            _row("T", 2, 0.5, 0.5, 0.5, 1, 4.0),
        ]
        m = F.compute_metrics(rows)
        # トップ=馬番1(3 着) → 単勝・連対は非的中、複勝のみ的中。
        self.assertAlmostEqual(m["win_hit"], 0.0)
        self.assertAlmostEqual(m["place_hit"], 0.0)
        self.assertAlmostEqual(m["show_hit"], 1.0)

    def test_missing_finishing_position_counts_as_non_hit(self):
        # 着順欠落（DNF/未確定）はトップでも全非的中、Brier は y=0 で計上。
        rows = [_row("X", 1, 0.4, 0.4, 0.4, None, 3.0)]
        m = F.compute_metrics(rows)
        self.assertAlmostEqual(m["win_hit"], 0.0)
        self.assertAlmostEqual(m["show_hit"], 0.0)
        self.assertAlmostEqual(m["brier_win"], (0.4 - 0.0) ** 2)
        # win_odds はあるが非的中 → 回収率は 0、母数は 1。
        self.assertEqual(m["payout_races"], 1)
        self.assertAlmostEqual(m["payout_rate"], 0.0)

    def test_missing_win_odds_excluded_from_payout_denominator(self):
        rows = [_row("Y", 1, 0.9, 0.9, 0.9, 1, None)]
        m = F.compute_metrics(rows)
        self.assertEqual(m["payout_races"], 0)
        self.assertIsNone(m["payout_rate"])


class ParseAndCompareTest(unittest.TestCase):
    REPORT = """# バックテスト 2026-06-13 〜 2026-06-14
評価レース数              : 72
単勝的中率               :  25.0%
連対的中率               :  44.4%
複勝的中率               :  61.1%
想定回収率               :  54.7%  (母数 72 レース)

## 確率校正（小さいほど良い）
種別            Brier    LogLoss
単勝           0.0589     0.2104
連対           0.1107     0.3690
複勝           0.1496     0.4682
"""

    def test_parse_backtest_report(self):
        e = F.parse_backtest_report(self.REPORT)
        self.assertEqual(e["races"], 72)
        self.assertAlmostEqual(e["win_hit"], 0.25)
        self.assertAlmostEqual(e["place_hit"], 0.444)
        self.assertAlmostEqual(e["show_hit"], 0.611)
        self.assertAlmostEqual(e["payout_rate"], 0.547)
        self.assertAlmostEqual(e["brier_win"], 0.0589)
        self.assertAlmostEqual(e["logloss_show"], 0.4682)

    def test_compare_passes_within_rounding(self):
        # 印字桁（%1桁・Brier4桁）の丸め差は許容内で一致扱い。
        computed = {
            "races": 72,
            "win_hit": 0.2503,
            "place_hit": 0.4444,
            "show_hit": 0.6111,
            "payout_rate": 0.5474,
            "brier_win": 0.05891,
            "brier_place": 0.11069,
            "brier_show": 0.14962,
            "logloss_win": 0.21041,
            "logloss_place": 0.36899,
            "logloss_show": 0.46821,
        }
        expected = F.parse_backtest_report(self.REPORT)
        self.assertEqual(F.compare(computed, expected), [])

    def test_compare_flags_drift(self):
        expected = F.parse_backtest_report(self.REPORT)
        computed = dict(expected)
        computed["races"] = 72
        computed["win_hit"] = 0.30  # 印字 25.0% から乖離
        mismatches = F.compare(computed, expected)
        self.assertTrue(any(k == "win_hit" for k, *_ in mismatches))


if __name__ == "__main__":
    unittest.main()
