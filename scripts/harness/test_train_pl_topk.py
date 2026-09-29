"""train_pl_topk.py のユニットテスト（#720）。

対象: 着順から段の勝者を作る規約（同着・中止）/ 割引 PL の尤度と勾配（数値微分）/
合成データからのパラメータ復元 / k=1・2 の λ 既定値 / 予測（softmax）。
"""

import sys

try:
    import numpy as np
    import pytest
except ImportError:
    if __name__ == "__main__":
        print("skip: numpy/pytest 不在（ローカルは pytest で実行）")
        sys.exit(0)
    raise

import os

import train_pl_topk as tp

# 評価器（prob_ledger）と同じモデルであることを突き合わせるため、predict-check を import パスに足す
sys.path.append(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "predict-check"))
import prob_ledger as pl  # noqa: E402


# ---------- 段の勝者（同着・中止の規約） ----------


def test_stage_winners_basic():
    # 行の並び: 着順 [3,1,2,5,4] → 1 着=idx1, 2 着=idx2, 3 着=idx0
    assert tp.stage_winners([3, 1, 2, 5, 4], k=3) == [1, 2, 0]
    assert tp.stage_winners([3, 1, 2, 5, 4], k=5) == [1, 2, 0, 4, 3]
    assert tp.stage_winners([3, 1, 2, 5, 4], k=1) == [1]


def test_stage_winners_truncates_at_ties_and_gaps():
    # 3 着同着: 1・2 着までで打ち切る
    assert tp.stage_winners([1, 2, 3, 3, 5], k=5) == [0, 1]
    # 1 着同着: 段が作れない
    assert tp.stage_winners([1, 1, 3, 4], k=3) == []
    # 3 着が欠落（中止など）: 2 着までで打ち切る
    assert tp.stage_winners([1, 2, None, 4], k=3) == [0, 1]
    # k が頭数より大きい
    assert tp.stage_winners([2, 1], k=5) == [1, 0]


# ---------- 尤度と勾配 ----------


def _random_races(rng, n_races=40, n_feat=3, k=3):
    races = []
    for _ in range(n_races):
        n = int(rng.integers(5, 12))
        x = rng.normal(size=(n, n_feat))
        fins = list(rng.permutation(n) + 1)
        if rng.random() < 0.2:
            fins[int(rng.integers(0, n))] = None  # 中止馬（分母には残る）
        races.append((x, tp.stage_winners(fins, k)))
    return tp.pack(races, n_feat)


def test_gradient_matches_numeric():
    rng = np.random.default_rng(0)
    data = _random_races(rng, k=3)
    theta = np.array([0.3, -0.2, 0.5, 0.8, 0.6])  # β(3) + λ2 + λ3
    f0, g = tp.neg_log_lik(theta, data, k=3, l2=0.1)
    eps = 1e-6
    num = np.zeros_like(theta)
    for i in range(len(theta)):
        t = theta.copy()
        t[i] += eps
        fp, _ = tp.neg_log_lik(t, data, k=3, l2=0.1)
        t[i] -= 2 * eps
        fm, _ = tp.neg_log_lik(t, data, k=3, l2=0.1)
        num[i] = (fp - fm) / (2 * eps)
    assert np.allclose(g, num, rtol=1e-4, atol=1e-5)


def test_single_stage_equals_conditional_logit():
    # k=1 の尤度は勝者だけの条件付きロジット: −Σ ln softmax(u)_winner
    x = np.array([[1.0, 0.0], [0.0, 1.0], [0.5, 0.5]])
    data = tp.pack([(x, [2])], 2)
    beta = np.array([0.4, -0.3])
    u = x @ beta
    expected = -(u[2] - np.log(np.exp(u).sum()))
    f, _ = tp.neg_log_lik(np.concatenate([beta, [1.0, 1.0]]), data, k=1, l2=0.0)
    assert f == pytest.approx(expected)


def test_discounted_stage_matches_harville_lambda():
    # 2 着段は p^λ2 / Σ_残 p^λ2（p = softmax(u)）と同じ（prob_ledger の割引 Harville と一致する形）
    x = np.array([[1.0], [0.2], [-0.5], [0.0]])
    beta = np.array([0.9])
    lam2 = 0.7
    data = tp.pack([(x, [0, 2])], 1)
    u = (x @ beta)
    p = np.exp(u) / np.exp(u).sum()
    rest = [1, 2, 3]
    expected = -(np.log(p[0]) + np.log(p[2] ** lam2 / sum(p[j] ** lam2 for j in rest)))
    f, _ = tp.neg_log_lik(np.array([0.9, lam2, 1.0]), data, k=2, l2=0.0)
    assert f == pytest.approx(expected)


def test_stages_beyond_third_share_lambda3():
    # 4 着段も λ3（3 着段と共有）。prob_ledger は 3 着までしか使わないが、学習の尤度は k=4/5 で効く
    x = np.array([[1.0], [0.2], [-0.5], [0.0], [0.4]])
    beta = np.array([0.9])
    lam2, lam3 = 0.7, 0.5
    order = [0, 2, 1, 4]
    data = tp.pack([(x, order)], 1)
    p = np.exp(x @ beta) / np.exp(x @ beta).sum()
    expected, rest = 0.0, list(range(5))
    for lam, j in zip((1.0, lam2, lam3, lam3), order):
        expected -= np.log(p[j] ** lam / sum(p[i] ** lam for i in rest))
        rest.remove(j)
    f, _ = tp.neg_log_lik(np.array([0.9, lam2, lam3]), data, k=4, l2=0.0)
    assert f == pytest.approx(expected)


def test_fit_recovers_parameters():
    rng = np.random.default_rng(1)
    beta_true = np.array([0.8, -0.5])
    lam2, lam3 = 0.7, 0.5
    races = []
    for _ in range(1500):
        n = 10
        x = rng.normal(size=(n, 2))
        u = x @ beta_true
        remaining = list(range(n))
        order = []
        for s in range(3):
            lam = (1.0, lam2, lam3)[s]
            w = np.exp(lam * u[remaining])
            j = remaining[int(rng.choice(len(remaining), p=w / w.sum()))]
            order.append(j)
            remaining.remove(j)
        fins = [None] * n
        for pos, j in enumerate(order, 1):
            fins[j] = pos
        nxt = 4
        for j in range(n):
            if fins[j] is None:
                fins[j] = nxt
                nxt += 1
        races.append((x, tp.stage_winners(fins, 3)))
    model = tp.fit(tp.pack(races, 2), k=3, l2=0.0)
    assert np.allclose(model.beta, beta_true, atol=0.1)
    assert model.lam2 == pytest.approx(lam2, abs=0.1)
    assert model.lam3 == pytest.approx(lam3, abs=0.1)


def test_lambda_defaults_when_not_identifiable():
    rng = np.random.default_rng(2)
    m1 = tp.fit(_random_races(rng, k=1), k=1, l2=0.1)
    assert m1.lam2 == 1.0 and m1.lam3 == 1.0
    m2 = tp.fit(_random_races(rng, k=2), k=2, l2=0.1)
    assert m2.lam3 == 1.0 and m2.lam2 != 1.0


def test_predict_is_softmax_of_utility():
    x = np.array([[1.0, 0.0], [0.0, 1.0], [0.5, 0.5]])
    model = tp.Model(beta=np.array([0.4, -0.3]), lam2=0.8, lam3=0.6)
    # u = (0.4, −0.3, 0.05): e^u = (1.491825, 0.740818, 1.051271)・和 3.283914 の手計算（λ は単勝確率に掛けない）
    assert np.allclose(tp.predict_race(model, x), [0.454283, 0.225590, 0.320127], atol=1e-6)
    # 効用が大きくてもオーバーフローしない
    big = tp.predict_race(tp.Model(beta=np.array([1000.0, 0.0])), x)
    assert np.all(np.isfinite(big)) and big[0] == pytest.approx(1.0)


def test_likelihood_matches_evaluator_combo_probs():
    # 学習の割引 PL と評価器の割引 Harville が同じモデル: 1-2-3 着の順序確率が一致する
    x = np.array([[1.0], [0.2], [-0.5], [0.0], [0.4]])
    beta, lam2, lam3 = np.array([0.9]), 0.7, 0.55
    order = [3, 0, 4]
    f, _ = tp.neg_log_lik(np.array([0.9, lam2, lam3]), tp.pack([(x, order)], 1), k=3, l2=0.0)
    combo = pl.combo_probs(tp.predict_race(tp.Model(beta=beta), x), lam2, lam3)
    assert np.exp(-f) == pytest.approx(combo.ordered3(*order))


# ---------- walk-forward ----------


def _feature_races(rng, dates, n_feat=2, beta=(0.8, -0.5)):
    """{race_id: Race} の合成データ（効用 x·β で着順を引く）。"""
    races = {}
    for i, d in enumerate(dates):
        n = 8
        x = rng.normal(size=(n, n_feat)) * 2.0 + 5.0  # 標準化が効くよう平均・スケールをずらす
        u = ((x - 5.0) / 2.0) @ np.array(beta)
        order = np.argsort(-(u + rng.gumbel(size=n)))
        fins = [0] * n
        for pos, j in enumerate(order, 1):
            fins[j] = pos
        races[f"R{i:04d}"] = tp.Race(date=d, horse_nums=list(range(1, n + 1)), x=x, fins=fins)
    return races


def test_walk_forward_trains_only_on_earlier_races(monkeypatch):
    rng = np.random.default_rng(3)
    dates = ["2025-03-10"] * 30 + ["2025-04-10"] * 30 + ["2025-05-10"] * 30
    races = _feature_races(rng, dates)
    seen = []
    real_fit = tp.fit

    def spy(data, k, l2, fix_lambda=False):
        seen.append(int((data["win"][:, 0] >= 0).sum()))
        return real_fit(data, k, l2, fix_lambda=fix_lambda)

    monkeypatch.setattr(tp, "fit", spy)
    out, meta = tp.walk_forward(races, "2025-03-01", "2025-04-01", "2025-05-31", k=3, l2=0.1, min_train=1)
    assert seen == [30, 60]  # 4 月は 3 月だけ、5 月は 3〜4 月で学習
    assert [m["month"] for m in meta] == ["2025-04", "2025-05"]
    assert {r["race_id"] for r in out} == {rid for rid, r in races.items() if r.date >= "2025-04-01"}


def test_walk_forward_output_is_normalized_with_model_lambda():
    rng = np.random.default_rng(4)
    races = _feature_races(rng, ["2025-03-10"] * 200 + ["2025-04-10"] * 5)
    out, meta = tp.walk_forward(races, "2025-03-01", "2025-04-01", "2025-04-30", k=3, l2=0.1)
    by_race = {}
    for r in out:
        by_race.setdefault(r["race_id"], []).append(r)
    for rows in by_race.values():
        assert sum(r["p_win"] for r in rows) == pytest.approx(1.0)
        assert {(r["lam2"], r["lam3"]) for r in rows} == {(meta[0]["lam2"], meta[0]["lam3"])}
    # 標準化は学習窓の平均・SD で行い、β は標準化後の尺度（真値 0.8/−0.5 の近く）
    assert meta[0]["beta"][0] > 0.3 and meta[0]["beta"][1] < -0.1
    fixed, meta_f = tp.walk_forward(races, "2025-03-01", "2025-04-01", "2025-04-30", k=3, l2=0.1, fix_lambda=True)
    assert meta_f[0]["lam2"] == 1.0 and meta_f[0]["lam3"] == 1.0
    assert all(r["lam2"] == 1.0 for r in fixed)


def _write_features(tmp_path, rows, extra_cols=()):
    import pl_features as pf
    head = ["race_id", "date", "horse_num", "horse_name", "finishing_position", "status", *pf.FEATURES, *extra_cols]
    lines = ["\t".join(head)]
    for rid, d, h, fin, val in rows:
        lines.append("\t".join([rid, d, str(h), f"H{h}", str(fin), "finished"]
                               + [val] * len(pf.FEATURES) + ["9.9"] * len(extra_cols)))
    p = tmp_path / "feats.tsv"
    p.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return str(p)


def test_load_features_uses_allowlist_and_validates(tmp_path):
    ok = [("R1", "2025-03-01", 1, 1, "0.5"), ("R1", "2025-03-01", 2, 2, "0.1")]
    races, cols = tp.load_features(_write_features(tmp_path, ok, extra_cols=("odds",)))
    assert "odds" not in cols and races["R1"].x.shape == (2, len(cols))  # 後から結合した列は使わない
    with pytest.raises(ValueError, match="許可リスト"):
        tp.load_features(_write_features(tmp_path, ok, extra_cols=("odds",)), ["n_runs", "odds"])
    bad = [
        ([*ok, ("R1", "2025-03-01", 2, 3, "0.2")], "重複"),
        ([ok[0], ("R1", "2025-03-02", 2, 2, "0.1")], "date"),
        ([ok[0], ("R1", "2025-03-01", 2, 2, "nan")], "有限"),
    ]
    for rows, msg in bad:
        with pytest.raises(ValueError, match=msg):
            tp.load_features(_write_features(tmp_path, rows))


def test_walk_forward_standardizes_with_training_window_only(monkeypatch):
    # 予測月だけ特徴量の分布を大きくずらす。標準化に予測月（未来）の行が混ざれば平均がずれる
    rng = np.random.default_rng(5)
    races = _feature_races(rng, ["2025-03-10"] * 60 + ["2025-04-10"] * 20)
    for r in races.values():
        if r.date >= "2025-04-01":
            r.x = r.x + 100.0
    seen = []
    real = tp.fit_scaler

    def spy(x):
        seen.append((x.shape[0], float(x.mean())))
        return real(x)

    monkeypatch.setattr(tp, "fit_scaler", spy)
    tp.walk_forward(races, "2025-03-01", "2025-04-01", "2025-04-30", k=3, l2=0.1)
    assert seen[0][0] == 60 * 8 and seen[0][1] < 10  # 3 月の 60 レース × 8 頭だけ（平均は 5 前後）


def test_walk_forward_skips_months_with_too_few_training_races():
    rng = np.random.default_rng(6)
    races = _feature_races(rng, ["2025-03-10"] * 10 + ["2025-04-10"] * 60 + ["2025-05-10"] * 5)
    out, meta = tp.walk_forward(races, "2025-03-01", "2025-04-01", "2025-05-31", k=3, l2=0.1, min_train=50)
    assert meta[0]["month"] == "2025-04" and "10 < 50" in meta[0]["skipped"]
    assert {r["race_id"] for r in out} == {rid for rid, r in races.items() if r.date >= "2025-05-01"}
    assert "skipped" not in meta[1]


def test_walk_forward_reports_month_when_fit_fails(monkeypatch):
    rng = np.random.default_rng(7)
    races = _feature_races(rng, ["2025-03-10"] * 60 + ["2025-04-10"] * 5)

    def boom(*a, **kw):
        raise RuntimeError("PL の最適化が収束しなかった")

    monkeypatch.setattr(tp, "fit", boom)
    with pytest.raises(RuntimeError, match="2025-04: PL の最適化"):
        tp.walk_forward(races, "2025-03-01", "2025-04-01", "2025-04-30", k=3, l2=0.1)


def test_lambda_bounds_match_the_evaluator():
    # 学習した λ を評価器が必ず受け取れる（片方だけ広げると TSV が拒否される）
    assert tp.LAM_BOUNDS == pl.TSV_LAM_RANGE


def test_standardize_uses_train_stats_and_guards_constant_columns():
    x = np.array([[1.0, 5.0], [3.0, 5.0]])
    mu, sd = tp.fit_scaler(x)
    assert np.allclose(mu, [2.0, 5.0]) and sd[1] == 1.0  # 定数列は割らない
    assert np.allclose(tp.apply_scaler(np.array([[2.0, 7.0]]), mu, sd), [[0.0, 2.0]])


def test_write_probs_full_precision(tmp_path):
    p = tmp_path / "probs.tsv"
    rows = [{"race_id": "R1", "horse_num": 1, "p_win": 1 / 3, "lam2": 0.7, "lam3": 0.55}]
    tp.write_probs(str(p), rows)
    text = p.read_text(encoding="utf-8").splitlines()
    assert text[0].split("\t") == ["race_id", "horse_num", "p_win", "lam2", "lam3"]
    assert float(text[1].split("\t")[2]) == 1 / 3  # repr で全桁


if __name__ == "__main__":
    sys.exit(pytest.main([__file__, "-q"]))
