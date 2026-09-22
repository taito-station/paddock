"""mag_sweep.py のユニットテスト。

中核は「再構成がパイプライン（blend → win_power）の独立実装と一致するか」。
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

import mag_sweep as ms


def _pipeline_reference(est: np.ndarray, odds: list, alpha: float, gamma: float) -> np.ndarray:
    """本番パイプラインの独立実装（blend_with_market_win + apply_win_power の写経・検算用）。"""
    e_hat = est / est.sum()
    implied = np.array([1.0 / o if o is not None else np.nan for o in odds])
    over = np.nansum(implied)
    if alpha >= 1.0 or over <= 0.0:
        mixed = e_hat.copy()
    else:
        q = implied / over
        mixed = np.where(np.isfinite(q), alpha * e_hat + (1.0 - alpha) * q, e_hat)
        mixed = np.minimum(mixed / mixed.sum(), 1.0)
    if gamma == 1.0:
        return mixed
    p = np.power(mixed, gamma)
    return np.minimum(p / p.sum(), 1.0)


def _rows(est: np.ndarray, odds: list, fin: list) -> list[dict]:
    """est から dump 相当の行を作る（u = normalize(est^γ0) = pure 列）。"""
    u = np.power(est, ms.GAMMA0)
    u = u / u.sum()
    ref = _pipeline_reference(est, odds, 0.2, 1.25)  # dump の model_win 列相当
    return [
        {
            "race_id": "R1",
            "horse_num": i + 1,
            "model_win_pure": float(u[i]),
            "model_win": float(ref[i]),
            "win_odds": odds[i],
            "finishing_position": fin[i],
        }
        for i in range(len(est))
    ]


def test_reconstruct_matches_pipeline_reference_across_grid():
    rng = np.random.default_rng(42)
    for _ in range(20):
        n = int(rng.integers(6, 16))
        est = rng.dirichlet(np.ones(n) * 1.5)
        odds = [float(o) for o in rng.uniform(1.2, 80.0, n)]
        rows = _rows(est, odds, [None] * n)
        for alpha in ms.ALPHAS:
            for gamma in ms.GAMMAS:
                recon = ms.reconstruct_win(rows, alpha, gamma)
                ref = _pipeline_reference(est, odds, alpha, gamma)
                assert np.max(np.abs(recon - ref)) < 1e-9, f"(α={alpha}, γ={gamma})"


def test_reconstruct_oddsless_horse_keeps_model_value_branch():
    # オッズ欠落馬は model 値据置 → 全体再正規化（blend_with_market_win の分岐再現）。
    est = np.array([0.5, 0.3, 0.2])
    odds = [2.0, 3.0, None]
    rows = _rows(est, odds, [1, 2, 3])
    recon = ms.reconstruct_win(rows, 0.2, 1.25)
    ref = _pipeline_reference(est, odds, 0.2, 1.25)
    assert np.max(np.abs(recon - ref)) < 1e-12


def test_fidelity_split_keeps_consistent_and_excludes_mismatch():
    est = np.array([0.4, 0.35, 0.25])
    ok_rows = _rows(est, [1.8, 3.5, 6.0], [1, 2, 3])
    bad_rows = _rows(est, [1.8, 3.5, 6.0], [1, 2, 3])
    # blend が実質 no-op だったレース（blended==pure）を模擬 → 再構成不能クラス。
    for r in bad_rows:
        r["model_win"] = r["model_win_pure"]
    ok, excluded, worst = ms.fidelity_split({"OK": ok_rows, "BAD": bad_rows})
    assert list(ok.keys()) == ["OK"]
    assert excluded == ["BAD"]
    assert worst < 1e-12


def test_candidate_metrics_alpha_zero_equals_market():
    # α=0 の再構成は市場含意確率（γ=1.0）に一致し、top1 は市場 1 番人気の的中で決まる。
    est = np.array([0.1, 0.6, 0.3])  # モデルは 2 番手推し
    odds = [1.5, 6.0, 4.0]  # 市場は 1 番手推し
    rows = _rows(est, odds, [1, 2, 3])
    races = {"R1": rows}
    met = ms.candidate_metrics(races, 0.0, 1.0)
    implied = np.array([1 / 1.5, 1 / 6.0, 1 / 4.0])
    q = implied / implied.sum()
    assert met["winner_probs"][0][0] == pytest.approx(q[0])
    assert met["top1"] == 1.0  # 市場 1 番人気（馬 1）が勝者


def test_is_market_identical_requires_gamma_one():
    # α=0 でも γ≠1.0 は normalize(q^γ) ≠ q なので恒等ではない（ゲート (b) をスキップしない）。
    assert ms.is_market_identical(0.0, 1.0)
    assert not ms.is_market_identical(0.0, 1.25)
    assert not ms.is_market_identical(0.2, 1.0)
    # 実データ検算: α=0, γ=1.25 の再構成は市場含意確率と一致しない。
    est = np.array([0.4, 0.35, 0.25])
    odds = [1.8, 3.5, 6.0]
    rows = _rows(est, odds, [1, 2, 3])
    recon = ms.reconstruct_win(rows, 0.0, 1.25)
    implied = np.array([1 / o for o in odds])
    q = implied / implied.sum()
    assert np.max(np.abs(recon - q)) > 1e-3


def test_judge_gates():
    # (a) CI 下限 > 0 / (b) None は自動成立・下限 ≥ −0.001 / (c) Brier +0.0002 許容。
    assert ms.judge_gates(0.002, None, 0.0578, 0.0580) == (True, True, True, True)
    assert ms.judge_gates(0.0, None, 0.0578, 0.0580)[0] is False  # 下限ちょうど 0 は FAIL
    assert ms.judge_gates(0.002, -0.001, 0.0578, 0.0580)[1] is True  # 境界は PASS
    assert ms.judge_gates(0.002, -0.0011, 0.0578, 0.0580)[1] is False
    assert ms.judge_gates(0.002, None, 0.05821, 0.0580)[2] is False  # Brier 許容超過
    assert ms.judge_gates(0.002, -0.002, 0.0578, 0.0580)[3] is False  # 1 本でも FAIL なら棄却


def test_tie_break_prefers_closest_to_current():
    # 同率のとき α → γ → m の辞書順で現行 (m10, 0.2, 1.25) に近い候補が先頭に来る。
    tied = [
        ("m5", 0.0, 1.0, 0.0, None),
        ("m10", 0.2, 1.25, 0.0, None),
        ("m10", 0.3, 1.25, 0.0, None),
    ]
    tied.sort(key=lambda r: ms.distance_to_current(r[0], r[1], r[2]))
    assert tied[0][:3] == ("m10", 0.2, 1.25)


if __name__ == "__main__":
    # 自走式（CI の predict-check ジョブと同じ `python3 <file>` 実行）。
    sys.exit(pytest.main([__file__, "-q"]))
