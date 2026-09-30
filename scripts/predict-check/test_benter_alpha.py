"""benter_alpha.py のユニットテスト（#722）。

対象: 市場確率の正規化（ワイドの中点・3連複の無投票の下限）/ 母集合と除外理由 / 条件付きロジットの推定
（合成データで既知の α・β を再現）/ 擬似 R² の一様基準 / 対応ありのブートストラップ / 窓と記録先の検査。
"""

import math
import sys
from collections import Counter

try:
    import numpy as np
    import pytest
except ImportError:
    if __name__ == "__main__":
        print("skip: numpy/pytest 不在（ローカルは pytest で実行）")
        sys.exit(0)
    raise

import benter_alpha as ba

LEDGER = str(ba.REPO_ROOT / "docs-original" / "722-benter-alpha-exotics.md")  # 入口の検査で止まるので書き込まない


# ---------- 市場確率 ----------


def test_market_probs_remove_takeout_by_normalizing():
    pi, n_floor = ba.market_probs([(1, 2), (1, 3), (2, 3)], {(1, 2): 2.0, (1, 3): 4.0, (2, 3): 4.0}, floor=False)
    assert pi == pytest.approx([0.5, 0.25, 0.25])
    assert n_floor == 0


def test_wide_uses_midpoint_of_band():
    rows = [{"combination_key": "1-2", "odds": "1.5", "odds_high": "2.5"}]
    assert ba.odds_value(rows[0], "wide") == 2.0
    assert ba.odds_value(rows[0], "wide", wide_low=True) == 1.5
    assert ba.odds_value({"odds": "12.0", "odds_high": ""}, "quinella") == 12.0


def test_unsold_trio_combo_gets_half_of_minimum_then_renormalized():
    combos = [(1, 2, 3), (1, 2, 4), (1, 3, 4), (2, 3, 4)]
    odds = {(1, 2, 3): 2.0, (1, 2, 4): 4.0, (1, 3, 4): 8.0}  # (2,3,4) は票なし
    pi, n_floor = ba.market_probs(combos, odds, floor=True)
    raw = np.array([0.5, 0.25, 0.125, 0.0625])  # 売れた 3 組は 1/O、無投票は最小（0.125）の半分
    assert pi == pytest.approx(raw / raw.sum())
    assert n_floor == 1


def test_unsold_combo_without_floor_is_dropped():
    combos = [(1, 2, 3), (1, 2, 4)]
    pi, _ = ba.market_probs(combos, {(1, 2, 3): 2.0}, floor=False)
    assert pi == pytest.approx([1.0, 0.0])


# ---------- 母集合 ----------


def _race(n=4, fins=(1, 2, 3, 4)):
    horses = list(range(1, n + 1))
    probs = {h: 1.0 / n for h in horses}
    return horses, probs, dict(zip(horses, fins))


def _all_odds(horses, k, o=10.0):
    import itertools
    return {c: o for c in itertools.combinations(horses, k)}


def test_build_race_quinella_winner_and_uniform_market():
    horses, probs, fins = _race()
    r = ba.build_race("quinella", horses, probs, (1.0, 1.0), fins, _all_odds(horses, 2), floor=True)
    assert r.reason is None
    assert len(r.combos) == 6 and r.winners == [r.combos.index((1, 2))]
    assert r.log_pi == pytest.approx(np.log(np.full(6, 1 / 6)))
    assert r.log_f == pytest.approx(np.log(np.full(6, 1 / 6)))  # 等確率の馬なら馬連も等確率


def test_build_race_wide_has_three_winners():
    horses, probs, fins = _race()
    r = ba.build_race("wide", horses, probs, (1.0, 1.0), fins, _all_odds(horses, 2), floor=True)
    assert sorted(r.combos[i] for i in r.winners) == [(1, 2), (1, 3), (2, 3)]


def test_build_race_exclusion_reasons():
    horses, probs, fins = _race()
    odds = _all_odds(horses, 2)
    tie = fins | {3: 2}  # 2 着の同着（1 着は一意）
    assert ba.build_race("quinella", horses, probs, (1, 1), tie, odds, floor=True).reason == "podium_not_unique"
    assert ba.build_race("quinella", horses, probs, (1, 1), fins, {}, floor=True).reason == "no_odds"
    odds_extra = odds | {(1, 5): 10.0}
    assert ba.build_race("quinella", horses, probs, (1, 1), fins, odds_extra, floor=True).reason == "market_horse_not_in_starters"
    missing = {h: p for h, p in probs.items() if h != 4}
    assert ba.build_race("quinella", horses, missing, (1, 1), fins, odds, floor=True).reason == "probs_missing"


def test_build_race_counts_trio_winner_unsold():
    horses, probs, fins = _race()
    odds = _all_odds(horses, 3)
    del odds[(1, 2, 3)]  # 的中した組に票がない
    r = ba.build_race("trio", horses, probs, (1, 1), fins, odds, floor=True)
    assert r.reason is None and r.winner_unsold
    r2 = ba.build_race("trio", horses, probs, (1, 1), fins, odds, floor=False)
    assert r2.reason == "winner_unsold"


# ---------- 推定（合成データ） ----------


def _synthetic(alpha, beta, n_races, n_combos, n_winners, seed):
    rng = np.random.default_rng(seed)
    races = []
    for _ in range(n_races):
        lf = np.log(rng.dirichlet(np.ones(n_combos)))
        lp = 0.6 * lf + 0.4 * np.log(rng.dirichlet(np.ones(n_combos)))  # 市場は f と相関させる
        lp -= np.log(np.exp(lp).sum())
        z = alpha * lf + beta * lp
        p = np.exp(z - z.max())
        p /= p.sum()
        winners = list(rng.choice(n_combos, size=n_winners, p=p))  # 複合尤度の検算用に独立に引く
        races.append(ba.Race(combos=list(range(n_combos)), log_f=lf, log_pi=lp, winners=winners))
    return races


@pytest.mark.parametrize("n_winners", [1, 3])
def test_fit_recovers_known_alpha_beta(n_winners):
    races = _synthetic(0.3, 0.8, 3000, 15, n_winners, seed=7)
    theta, se, _ = ba.fit(ba.pack(races))
    assert theta == pytest.approx([0.3, 0.8], abs=0.06)
    assert np.all(se > 0) and np.all(se < 0.1)
    if n_winners == 1:  # 真値が ±3SE に入る（カテゴリカルは Hessian SE が正しい尺度）
        assert np.all(np.abs(theta - [0.3, 0.8]) < 3 * se)


def test_fit_alpha_zero_when_model_has_no_information():
    races = _synthetic(0.0, 1.0, 3000, 15, 1, seed=3)
    theta, se, _ = ba.fit(ba.pack(races))
    assert abs(theta[0]) < 3 * se[0]


def test_race_nll_matches_manual_softmax():
    lf = np.log(np.array([0.5, 0.3, 0.2]))
    lp = np.log(np.array([0.2, 0.3, 0.5]))
    races = [ba.Race(combos=[0, 1, 2], log_f=lf, log_pi=lp, winners=[1])]
    z = 0.4 * lf + 0.9 * lp
    expect = -(z[1] - math.log(np.exp(z).sum()))
    assert ba.race_nll(ba.pack(races), np.array([0.4, 0.9])) == pytest.approx([expect])


def test_market_only_nll_is_log_pi():
    lf = np.log(np.array([0.5, 0.3, 0.2]))
    lp = np.log(np.array([0.2, 0.3, 0.5]))
    races = [ba.Race(combos=[0, 1, 2], log_f=lf, log_pi=lp, winners=[2])]
    assert ba.race_nll(ba.pack(races), np.array([0.0, 1.0])) == pytest.approx([-math.log(0.5)])


# ---------- 評価（擬似 R²・対応ありブートストラップ） ----------


def test_uniform_prediction_has_zero_pseudo_r2_per_bet_type():
    for n_combos, n_winners in ((6, 1), (6, 3), (4, 1)):
        lz = np.log(np.full(n_combos, 1 / n_combos))
        races = [ba.Race(combos=list(range(n_combos)), log_f=lz, log_pi=lz, winners=list(range(n_winners)))]
        pk = ba.pack(races)
        nll = ba.race_nll(pk, np.array([0.0, 1.0]))
        assert ba.pseudo_r2(nll, ba.uniform_nll(pk)) == pytest.approx(0.0)
        assert ba.uniform_nll(pk)[0] == pytest.approx(n_winners * math.log(n_combos))


def test_paired_bootstrap_of_identical_systems_is_zero():
    rng = np.random.default_rng(0)
    a = rng.random(50)
    lo, hi = ba.paired_ci(a, a, np.ones(50), n_boot=200, seed=1)
    assert lo == 0.0 and hi == 0.0


def test_paired_bootstrap_sign():
    a = np.full(40, 2.0)
    b = np.full(40, 1.0)
    lo, hi = ba.paired_ci(a, b, np.ones(40), n_boot=200, seed=1)  # 平均の差 +1
    assert lo == pytest.approx(1.0) and hi == pytest.approx(1.0)


@pytest.mark.parametrize("n_winners", [1, 3])
def test_fit_beta_only_recovers_market_sharpening(n_winners):
    races = _synthetic(0.0, 1.3, 3000, 15, n_winners, seed=21)
    beta, _ = ba.fit_beta(ba.pack(races))
    assert beta == pytest.approx(1.3, abs=0.08)


def test_roi_threshold_uses_expected_hits_per_combo():
    # ワイド（的中 3 組）: p = 1/6 の 6 組、オッズ 2.5。的中数の期待は 3p = 0.5 → 期待値 1.25 > 1 なので全組買う
    lz = np.log(np.full(6, 1 / 6))
    race = ba.Race(combos=list(range(6)), log_f=lz, log_pi=lz, winners=[0, 1, 2], odds=np.full(6, 2.5))
    roi, n_bet, _, _ = ba.roi_reference(ba.pack([race]), np.array([0.0, 1.0]), n_boot=20, seed=1)
    assert n_bet == 6 and roi == pytest.approx(3 * 2.5 / 6)
    # 馬連（的中 1 組）: 期待値 p·O = 2.5/6 < 1 なので買わない
    q = ba.Race(combos=list(range(6)), log_f=lz, log_pi=lz, winners=[0], odds=np.full(6, 2.5))
    assert ba.roi_reference(ba.pack([q]), np.array([0.0, 1.0]), n_boot=20, seed=1)[1] == 0


# ---------- 窓と記録先 ----------


def test_ledger_must_be_existing_primary_doc(tmp_path):
    ok = ba.REPO_ROOT / "docs-original" / "722-benter-alpha-exotics.md"
    assert ba.validate_ledger(str(ok)) is None
    assert ba.validate_ledger("/dev/null") is not None
    assert ba.validate_ledger(str(tmp_path / "x.md")) is not None
    outside = tmp_path / "y.md"
    outside.write_text("", encoding="utf-8")
    assert ba.validate_ledger(str(outside)) is not None  # 実在しても docs-original/ の外は不可
    assert ba.validate_ledger(str(ba.REPO_ROOT / "docs-original" / "no-such.md")) is not None


def test_windows_follow_frozen_protocol():
    assert ba.validate_args("2025-07-01", "2025-12-31", "2026-01-01", "2026-08-31", "test", None, None) is not None
    assert ba.validate_args("2025-07-01", "2025-12-31", "2026-01-01", "2026-08-31", "test", "x.md", "lbl") is None
    assert ba.validate_args("2025-07-01", "2026-02-28", "2026-01-01", "2026-08-31", "dev", None, None) is not None
    assert ba.validate_args("2025-07-01", "2025-12-31", "2026-01-01", "2026-08-31", "dev", None, None) is None


def test_cli_rejects_test_window_without_ledger_and_sensitivity_on_test(tmp_path, monkeypatch):
    monkeypatch.setattr(ba.pl, "code_version", lambda: ("abc1234", False))  # 作業ツリーの状態に依らず入口の検査だけを見る
    base = ["--probs", "p", "--odds", "o", "--results", "r"]
    assert ba.main(base + ["--windows", "test"]) == 2
    assert ba.main(base + ["--windows", "test", "--ledger", LEDGER, "--label", "x", "--trio-no-floor"]) == 2
    assert ba.main(base + ["--windows", "test", "--ledger", LEDGER, "--label", "x", "--wide-low"]) == 2
    assert ba.main(base + ["--bet-types", "exacta"]) == 2
    assert ba.main(base + ["--windows", "test", "--ledger", "/dev/null", "--label", "x"]) == 2


def test_cli_rejects_test_window_on_dirty_tree(tmp_path, monkeypatch):
    monkeypatch.setattr(ba.pl, "code_version", lambda: ("abc1234", True))
    args = ["--probs", "p", "--odds", "o", "--results", "r", "--windows", "test",
            "--ledger", LEDGER, "--label", "x"]
    assert ba.main(args) == 2


def test_judge_truth_table():
    assert ba.judge(0.1, 0.1, 0.001, -0.001) is True
    assert ba.judge(-0.1, 0.1, 0.001, -0.001) is False  # fit 窓の α の CI が 0 を跨ぐ
    assert ba.judge(0.1, -0.1, 0.001, -0.001) is False  # eval 窓で α の CI が 0 を跨ぐ
    assert ba.judge(0.1, 0.1, -0.001, -0.001) is False  # ΔR² の CI が 0 を跨ぐ
    assert ba.judge(0.1, 0.1, 0.001, 0.001) is False  # NLL 差の CI が 0 を跨ぐ


def test_judgement_requires_all_conditions():
    edge_fit = _synthetic(0.5, 0.8, 1500, 15, 1, seed=11)
    edge_eval = _synthetic(0.5, 0.8, 1500, 15, 1, seed=12)
    _, res = ba.report_bet("quinella", edge_fit, Counter(), edge_eval, Counter(), n_boot=100, seed=1)
    assert res["edge"] is True
    none_fit = _synthetic(0.0, 1.0, 1500, 15, 1, seed=13)
    none_eval = _synthetic(0.0, 1.0, 1500, 15, 1, seed=14)
    _, res = ba.report_bet("quinella", none_fit, Counter(), none_eval, Counter(), n_boot=100, seed=1)
    assert res["edge"] is False
    # fit では α>0 でも eval で情報が消えたら「なし」
    _, res = ba.report_bet("quinella", edge_fit, Counter(), none_eval, Counter(), n_boot=100, seed=1)
    assert res["edge"] is False


def test_end_to_end_dev_run(tmp_path, capsys):
    import itertools
    rng = np.random.default_rng(5)
    probs = ["race_id\thorse_num\tp_win\tlam2\tlam3"]
    odds = ["race_id\tbet_type\tcombination_key\todds\todds_high\tpopularity"]
    res = ["race_id\tdate\thorse_num\tstatus\tfinishing_position"]
    for r in range(40):
        rid = f"R{r}"
        p = rng.dirichlet(np.ones(6))
        order = rng.permutation(6) + 1
        for h in range(1, 7):
            probs.append(f"{rid}\t{h}\t{p[h - 1]}\t0.9\t0.8")
            res.append(f"{rid}\t2025-08-01\t{h}\tfinished\t{list(order).index(h) + 1}")
        for bt, k in (("quinella", 2), ("wide", 2), ("trio", 3)):
            for c in itertools.combinations(range(1, 7), k):
                o = 5 + rng.random() * 20
                odds.append(f"{rid}\t{bt}\t{'-'.join(map(str, c))}\t{o:.1f}\t{(o + 2 if bt == 'wide' else ''):}\t1")
    for name, rows in (("p.tsv", probs), ("o.tsv", odds), ("r.tsv", res)):
        (tmp_path / name).write_text("\n".join(rows) + "\n", encoding="utf-8")
    rc = ba.main(["--probs", str(tmp_path / "p.tsv"), "--odds", str(tmp_path / "o.tsv"),
                  "--results", str(tmp_path / "r.tsv"), "--bootstrap", "20"])
    out = capsys.readouterr().out
    assert rc == 0
    for bt in ("quinella", "wide", "trio"):
        assert f"### {bt}" in out
    assert "fit 窓: 40R" in out and "eval は測らない" in out


if __name__ == "__main__":
    sys.exit(pytest.main([__file__, "-q"]))
