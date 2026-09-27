"""blend_fit.py のユニットテスト。

中核は「既知の (A,B) で合成した勝者データから (Â,B̂) を回収できるか」（推定器の実証）。
CI の stdlib-only ジョブでは numpy/pytest 不在のため自己スキップする（test_prob_eval.py と同方針）。
"""

import sys

try:
    import numpy as np
    import pytest
except ImportError:
    if __name__ == "__main__":
        print("skip: numpy/pytest 不在（stdlib-only CI では対象外。ローカルは pytest で実行）")
        sys.exit(0)
    raise

import blend_fit as bf


def _synth(n_races: int, a: float, b: float, seed: int):
    """u, q を独立に引き、P ∝ u^a q^b から勝者を抽選した合成データ（fit_ab 入力形式）。"""
    rng = np.random.default_rng(seed)
    races = []
    for _ in range(n_races):
        n = int(rng.integers(8, 15))
        u = rng.dirichlet(np.ones(n) * 1.5)
        q = rng.dirichlet(np.ones(n) * 3.0)
        z = a * np.log(u) + b * np.log(q)
        p = np.exp(z - z.max())
        p /= p.sum()
        w = int(rng.choice(n, p=p))
        races.append((np.log(u), np.log(q), w))
    return races


def test_fit_recovers_known_exponents():
    races = _synth(4000, a=0.3, b=0.9, seed=42)
    theta, se, _ = bf.fit_ab(races)
    assert abs(theta[0] - 0.3) < 3 * se[0] + 0.02, f"Â={theta[0]} se={se[0]}"
    assert abs(theta[1] - 0.9) < 3 * se[1] + 0.02, f"B̂={theta[1]} se={se[1]}"
    assert se[0] < 0.1 and se[1] < 0.1  # 4000R で十分タイトな SE


def test_fit_market_only_data_yields_zero_model_exponent():
    races = _synth(4000, a=0.0, b=1.0, seed=7)
    theta, se, _ = bf.fit_ab(races)
    assert abs(theta[0]) < 3 * se[0] + 0.02
    assert abs(theta[1] - 1.0) < 3 * se[1] + 0.02


def test_loglik_prefers_true_theta():
    races = _synth(2000, a=0.5, b=0.8, seed=3)
    assert bf.loglik(races, np.array([0.5, 0.8])) > bf.loglik(races, np.array([0.0, 1.0]))
    assert bf.loglik(races, np.array([0.5, 0.8])) > bf.loglik(races, np.array([1.0, 0.0]))


def test_composite_winner_probs_matches_manual():
    rows = [
        {
            "race_id": "R1",
            "model_win_pure": u,
            "model_win": u,
            "win_odds": o,
            "finishing_position": fin,
        }
        for u, o, fin in [(0.5, 2.0, 1), (0.3, 3.0, 2), (0.2, 6.0, 3)]
    ]
    races = {"R1": rows}
    out = bf.composite_winner_probs(races, np.array([0.0, 1.0]))
    # (0,1) は市場そのもの: implied [.5, 1/3, 1/6] → q1 = .5
    assert out == [(pytest.approx(0.5), 3)]
    out2 = bf.composite_winner_probs(races, np.array([1.0, 0.0]))
    assert out2 == [(pytest.approx(0.5), 3)]  # (1,0) はモデル再正規化（Σ=1 なのでそのまま）


def test_logpool_races_excludes_zero_model_and_partial_odds():
    mk = lambda rid, u, o, fin: {
        "race_id": rid,
        "model_win_pure": u,
        "model_win": u,
        "win_odds": o,
        "finishing_position": fin,
    }
    races = {
        "OK": [mk("OK", 0.6, 2.0, 1), mk("OK", 0.4, 3.0, 2)],
        "ZERO": [mk("ZERO", 0.0, 2.0, 1), mk("ZERO", 1.0, 3.0, 2)],
        "NOODDS": [mk("NOODDS", 0.6, None, 1), mk("NOODDS", 0.4, 3.0, 2)],
    }
    out, excluded = bf.logpool_races(races)
    assert len(out) == 1
    assert excluded["zero_model"] == 1
    assert excluded["no_market"] == 1


if __name__ == "__main__":
    # 自走式（CI の predict-check ジョブと同じ `python3 <file>` 実行）。
    sys.exit(pytest.main([__file__, "-q"]))
