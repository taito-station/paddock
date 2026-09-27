"""harville_lambda_fit.py のユニットテスト。

中核は「既知の λ で合成した着順から λ̂ を回収できるか」（推定器の実証・変異で落ちる検査）。
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

import harville_lambda_fit as hf
import prob_eval as pe


def _synth_races(n_races: int, lam2: float, lam3: float, seed: int, key: str = "model_win_pure"):
    """discounted Harville の生成過程から着順を合成し、podium_races 形式で返す。

    1 着は素の p、2 着は残りから p^λ2 正規化、3 着は残りから p^λ3 正規化で抽選する。
    """
    rng = np.random.default_rng(seed)
    races = {}
    for r in range(n_races):
        n = int(rng.integers(8, 15))
        raw = rng.dirichlet(np.ones(n) * 1.2)
        rows = []
        for i in range(n):
            rows.append(
                {
                    "race_id": f"R{r}",
                    "horse_num": i + 1,
                    key: float(raw[i]),
                    "model_win": float(raw[i]),
                    "finishing_position": None,
                }
            )
        i1 = int(rng.choice(n, p=raw))
        rest2 = [i for i in range(n) if i != i1]
        w2 = raw[rest2] ** lam2
        i2 = int(rng.choice(rest2, p=w2 / w2.sum()))
        rest3 = [i for i in range(n) if i not in (i1, i2)]
        w3 = raw[rest3] ** lam3
        i3 = int(rng.choice(rest3, p=w3 / w3.sum()))
        rows[i1]["finishing_position"] = 1
        rows[i2]["finishing_position"] = 2
        rows[i3]["finishing_position"] = 3
        races[f"R{r}"] = (rows, i1, i2, i3)
    return races


def test_fit_recovers_known_lambda():
    races = _synth_races(3000, lam2=0.8, lam3=0.65, seed=42)
    lam2, ci2, imp2 = hf.fit_lambda(races, "model_win_pure", 2)
    lam3, ci3, imp3 = hf.fit_lambda(races, "model_win_pure", 3)
    assert abs(lam2 - 0.8) < 0.06, f"λ2 回収失敗: {lam2}"
    assert abs(lam3 - 0.65) < 0.06, f"λ3 回収失敗: {lam3}"
    assert ci2[0] < 0.8 < ci2[1]
    assert imp2 > 10.0  # λ=1 より明確に良い


def test_fit_identity_data_yields_lambda_near_one():
    # 95% CI の被覆そのものは構造的に 5% のシードで外れるため assert しない。
    # 固定するのは「λ=1 データで大きな偽改善・大きなズレが出ない」こと。
    races = _synth_races(3000, lam2=1.0, lam3=1.0, seed=7)
    lam2, ci2, imp = hf.fit_lambda(races, "model_win_pure", 2)
    assert abs(lam2 - 1.0) < 0.08, f"λ=1 データで λ̂={lam2}"
    assert ci2[0] <= lam2 <= ci2[1]
    assert imp < 4.0, f"λ=1 データで偽の logL 改善: {imp}"


def test_stage_loglik_prefers_true_lambda():
    races = _synth_races(1500, lam2=0.7, lam3=0.7, seed=3)
    ll_true = hf.stage_loglik(races, "model_win_pure", 2, 0.7)
    ll_one = hf.stage_loglik(races, "model_win_pure", 2, 1.0)
    assert ll_true > ll_one


def test_bucket_z_detects_overestimation_direction():
    # λ=0.7 で生成したデータを λ=1 で評価すると、高確率帯は過大評価（Z<0）・
    # 低確率帯は過小評価（Z>0）になる（Benter Table 9/10 の再現）。
    races = _synth_races(4000, lam2=0.7, lam3=0.7, seed=11)
    table = hf.bucket_z_table(races, "model_win_pure", 2, 1.0)
    assert table[-1]["z"] < -2.0, f"高確率帯 Z: {table[-1]['z']}"
    assert table[0]["z"] > 2.0, f"低確率帯 Z: {table[0]['z']}"
    # 真の λ で評価すればバケットの系統的偏りは消える（|Z| が縮む）。
    fixed = hf.bucket_z_table(races, "model_win_pure", 2, 0.7)
    assert abs(fixed[-1]["z"]) < 2.5
    assert abs(fixed[0]["z"]) < 2.5


def test_podium_races_excludes_non_unique():
    rows_ok = [
        {"race_id": "R1", "finishing_position": p, "model_win": 0.2, "model_win_pure": 0.2}
        for p in (1, 2, 3, 4)
    ]
    rows_dead = [
        {"race_id": "R2", "finishing_position": p, "model_win": 0.25, "model_win_pure": 0.25}
        for p in (1, 2, 2, 4)  # 2 着同着
    ]
    table = pe.build_race_table(rows_ok + rows_dead)
    podium, excluded = hf.podium_races(table.winner_races())
    assert list(podium.keys()) == ["R1"]
    assert excluded["no_unique_podium"] == 1


if __name__ == "__main__":
    # 自走式（CI の predict-check ジョブと同じ `python3 <file>` 実行）。
    sys.exit(pytest.main([__file__, "-q"]))
