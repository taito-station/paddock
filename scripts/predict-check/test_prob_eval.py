"""prob_eval.py のユニットテスト。

対象: ヘッダ名参照の dump 読み込み（NaN/日付検証含む）/ 市場確率の再構成 /
PAV(isotonic・tie プール) / CORP 分解（独立導出の期待値）/ Bolton-Chapman 擬似 R² と ΔR²
（異質データでの CI）/ consistency band の構造スモーク / fidelity の手計算一致 / 窓分割と
窓引数検証。乱数を使う機能（band・bootstrap CI）は seed 固定で構造的性質を検査する。
"""

import math
import os
import sys
import tempfile

# CI の predict-check ジョブ（stdlib-only・`python3 <file>` 直接実行）では numpy/pytest が
# 無いため自己スキップする（scripts/harness の numpy 依存テストを CI 対象外とする既存方針と
# 同じ扱い。ローカルでは `python3 -m pytest` または直接実行で全テストが走る）。
try:
    import numpy as np
    import pytest
except ImportError:
    if __name__ == "__main__":
        print("skip: numpy/pytest 不在（stdlib-only CI では対象外。ローカルは pytest で実行）")
        sys.exit(0)
    raise

import prob_eval as pe


# ---------- dump 読み込み（ヘッダ名参照） ----------

HEADER = (
    "race_id\tdate\thorse_num\tmodel_win\tmodel_place\tmodel_show\t"
    "finishing_position\twin_odds\tpopularity\t"
    "model_win_pure\tmodel_place_pure\tmodel_show_pure"
)


def _write_tsv(lines):
    d = tempfile.mkdtemp()
    p = os.path.join(d, "dump.tsv")
    with open(p, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    return p


def test_load_dump_by_header_name():
    # 列順が変わってもヘッダ名で引ける（末尾追加に依存しない）。
    p = _write_tsv(
        [
            HEADER,
            "R1\t2026-01-10\t1\t0.6\t0.8\t0.9\t1\t1.5\t1\t0.5\t0.7\t0.8",
            "R1\t2026-01-10\t2\t0.4\t0.6\t0.7\t2\t3.0\t2\t0.5\t0.7\t0.8",
        ]
    )
    rows = pe.load_dump(p)
    assert len(rows) == 2
    assert rows[0]["race_id"] == "R1"
    assert rows[0]["model_win"] == pytest.approx(0.6)
    assert rows[0]["model_win_pure"] == pytest.approx(0.5)
    assert rows[1]["finishing_position"] == 2
    assert rows[1]["win_odds"] == pytest.approx(3.0)


def test_load_dump_missing_required_column_raises():
    p = _write_tsv(["race_id\tdate\thorse_num\tmodel_win", "R1\t2026-01-10\t1\t0.5"])
    with pytest.raises(ValueError):
        pe.load_dump(p)


def test_load_dump_empty_cells_are_none():
    p = _write_tsv(
        [
            HEADER,
            "R1\t2026-01-10\t1\t0.6\t0.8\t0.9\t\t\t\t0.5\t0.7\t0.8",
        ]
    )
    rows = pe.load_dump(p)
    assert rows[0]["finishing_position"] is None
    assert rows[0]["win_odds"] is None


def test_load_dump_rejects_nan_with_row_number():
    # NaN/inf は黙って指標を汚染する（PAV の比較が常に False になる）ため即エラー。
    p = _write_tsv(
        [
            HEADER,
            "R1\t2026-01-10\t1\tnan\t0.8\t0.9\t1\t2.0\t1\t0.5\t0.7\t0.8",
        ]
    )
    with pytest.raises(ValueError, match=r":2 .*model_win"):
        pe.load_dump(p)


def test_load_dump_rejects_malformed_date():
    p = _write_tsv(
        [
            HEADER,
            "R1\t2026/01/10\t1\t0.6\t0.8\t0.9\t1\t2.0\t1\t0.5\t0.7\t0.8",
        ]
    )
    with pytest.raises(ValueError, match="date"):
        pe.load_dump(p)


# ---------- 市場確率の再構成 ----------

def test_market_probs_normalizes_implied():
    # q_i = (1/odds_i) / Σ(1/odds)。オッズ [2.0, 4.0, 4.0] → implied [.5, .25, .25] → q そのまま。
    race = [
        {"win_odds": 2.0},
        {"win_odds": 4.0},
        {"win_odds": 4.0},
    ]
    q = pe.market_probs(race)
    assert q == pytest.approx([0.5, 0.25, 0.25])


def test_market_probs_partial_coverage_returns_none():
    # 1 頭でもオッズ欠落・値域違反（<1.0）のレースは市場系統から除外（None）。
    assert pe.market_probs([{"win_odds": 2.0}, {"win_odds": None}]) is None
    assert pe.market_probs([{"win_odds": 2.0}, {"win_odds": 0.0}]) is None


# ---------- PAV（isotonic 回帰） ----------

def test_pav_output_is_monotone_and_calibrated_on_groups():
    # 逆転を含む入力でも出力は非減少。
    p = np.array([0.1, 0.2, 0.3, 0.4, 0.9])
    y = np.array([0.0, 1.0, 0.0, 0.0, 1.0])
    fitted = pe.pav(p, y)
    assert all(fitted[i] <= fitted[i + 1] + 1e-12 for i in range(len(fitted) - 1))
    # PAV は各プールで実測平均に一致する（総和保存）。
    assert fitted.sum() == pytest.approx(y.sum())


def test_pav_perfectly_separated_recovers_steps():
    p = np.array([0.1, 0.1, 0.9, 0.9])
    y = np.array([0.0, 0.0, 1.0, 1.0])
    fitted = pe.pav(p, y)
    assert fitted == pytest.approx([0.0, 0.0, 1.0, 1.0])


def test_pav_ties_are_pooled_and_order_invariant():
    # 同値 p は事前プールされ、同じ予測値には同じ CEP が付く（行順に依存しない）。
    p = np.array([0.5, 0.5])
    for y in ([0.0, 1.0], [1.0, 0.0]):
        fitted = pe.pav(p, np.array(y))
        assert fitted == pytest.approx([0.5, 0.5])


# ---------- CORP 分解（S̄ = MCB − DSC + UNC の恒等式） ----------

def test_corp_decomposition_calibrated_data_has_small_mcb():
    rng = np.random.default_rng(42)
    p = rng.uniform(0.05, 0.95, size=5000)
    y = (rng.uniform(size=5000) < p).astype(float)  # 完全校正データ
    d = pe.corp_decomposition(p, y)
    # 完全校正なら MCB ≈ 0（PAV の過適合ぶんだけ僅かに正）。
    assert 0.0 <= d["mcb"] < 0.005
    assert d["unc"] == pytest.approx(np.mean(y) * (1 - np.mean(y)), abs=1e-12)


def test_corp_decomposition_constant_prediction_has_zero_dsc():
    # 定数予測は識別力ゼロ: PAV あてはめは全体平均になり DSC = UNC − S̄(PAV) = 0。
    # MCB = Brier − S̄(PAV) = (p−ȳ)² は手計算どおり（独立導出の期待値）。
    p = np.full(8, 0.3)
    y = np.array([1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0])  # ȳ = 0.25
    d = pe.corp_decomposition(p, y)
    assert d["dsc"] == pytest.approx(0.0, abs=1e-12)
    assert d["mcb"] == pytest.approx((0.3 - 0.25) ** 2, abs=1e-12)
    assert d["unc"] == pytest.approx(0.25 * 0.75, abs=1e-12)


def test_corp_decomposition_perfect_separation_has_dsc_equal_unc():
    # 完全分離では PAV あてはめが y に一致し S̄(PAV)=0 → DSC = UNC、MCB = Brier。
    p = np.array([0.2, 0.3, 0.7, 0.8])
    y = np.array([0.0, 0.0, 1.0, 1.0])
    d = pe.corp_decomposition(p, y)
    assert d["dsc"] == pytest.approx(d["unc"], abs=1e-12)
    assert d["mcb"] == pytest.approx(d["brier"], abs=1e-12)


def test_corp_decomposition_detects_miscalibration():
    rng = np.random.default_rng(7)
    p = rng.uniform(0.05, 0.95, size=5000)
    y = (rng.uniform(size=5000) < p**2).astype(float)  # 系統的に過大予測
    d = pe.corp_decomposition(p, y)
    assert d["mcb"] > 0.01  # 明確な非校正を検出


# ---------- Bolton-Chapman 擬似 R² と ΔR² ----------

def test_r2_hand_computed():
    # 2 レース: R1 は 4 頭で勝者確率 0.5、R2 は 2 頭で勝者確率 0.8。
    # R² = 1 − [ln .5 + ln .8] / [ln .25 + ln .5]
    races = [(0.5, 4), (0.8, 2)]
    expected = 1.0 - (math.log(0.5) + math.log(0.8)) / (math.log(1 / 4) + math.log(1 / 2))
    assert pe.pseudo_r2(races) == pytest.approx(expected)


def test_r2_uniform_prediction_is_zero():
    races = [(1 / 8, 8), (1 / 12, 12)]
    assert pe.pseudo_r2(races) == pytest.approx(0.0, abs=1e-12)


def test_delta_r2_bootstrap_ci_contains_point_estimate():
    # 異質なレース集合（確率・頭数を散らす）で CI が点に縮退しないことも同時に確認する
    # （全レース同値だと再標本が常に同一集合になり分位計算を検証できない）。
    rng = np.random.default_rng(0)
    races_a = [(float(pw), int(n)) for pw, n in zip(rng.uniform(0.2, 0.7, 200), rng.integers(8, 18, 200))]
    races_b = [(pw * 0.6, n) for pw, n in races_a]  # 一様に劣る系統
    lo, hi = pe.delta_r2_ci(races_a, races_b, n_boot=200, seed=1)
    point = pe.pseudo_r2(races_a) - pe.pseudo_r2(races_b)
    assert lo <= point <= hi
    assert hi - lo > 0.0  # CI が点に縮退していない
    assert lo > 0.0  # 全レースで優位なので CI は 0 を跨がない


def test_pseudo_r2_empty_or_degenerate_raises():
    with pytest.raises(ValueError):
        pe.pseudo_r2([])
    with pytest.raises(ValueError):
        pe.pseudo_r2([(1.0, 1)])  # 1 頭立てのみ → 分母 0


def test_reliability_band_structure_on_calibrated_data():
    # 完全校正データの構造スモーク: band_lo ≤ band_hi、グリッド p は非減少、
    # 帯外点は少数（90% pointwise・10 点で H0 下の期待 ~1 点）。
    rng = np.random.default_rng(7)
    p = rng.uniform(0.05, 0.95, size=2000)
    y = (rng.uniform(size=2000) < p).astype(float)
    band = pe.reliability_band(p, y, n_boot=200, seed=3)
    assert len(band) == 10
    assert all(b["band_lo"] <= b["band_hi"] + 1e-12 for b in band)
    grid_p = [b["p"] for b in band]
    assert grid_p == sorted(grid_p)
    assert sum(1 for b in band if b["outside"]) <= 3


def test_fidelity_hand_computed():
    rows = [
        _mk_row("R1", "2026-01-10", 1, 0.6, 1, 2.0),
        _mk_row("R1", "2026-01-10", 2, 0.4, 2, 3.0),
        _mk_row("R2", "2026-01-10", 1, 0.5, 2, 2.0),
    ]
    fid = pe.fidelity(rows)
    expected = ((0.6 - 1) ** 2 + (0.4 - 0) ** 2 + (0.5 - 0) ** 2) / 3
    assert fid["win_brier"] == pytest.approx(expected)
    assert fid["races"] == 2
    assert fid["horses"] == 3


def test_validate_windows():
    assert pe.validate_windows(None, None) is None
    assert pe.validate_windows("2025-12-31", "2026-01-01") is None
    assert pe.validate_windows("2025-12-31", None) is not None  # 片方のみは拒否
    assert pe.validate_windows(None, "2026-01-01") is not None
    assert pe.validate_windows("2025/12/31", "2026-01-01") is not None  # 非 ISO
    assert pe.validate_windows("2026-01-01", "2026-01-01") is not None  # 重複窓


# ---------- レース集計・窓分割 ----------

def _mk_row(rid, date, num, win, fin, odds, pure=None):
    return {
        "race_id": rid,
        "date": date,
        "horse_num": num,
        "model_win": win,
        "model_place": win,
        "model_show": win,
        "model_win_pure": pure if pure is not None else win,
        "model_place_pure": pure if pure is not None else win,
        "model_show_pure": pure if pure is not None else win,
        "finishing_position": fin,
        "win_odds": odds,
        "popularity": None,
    }


def test_split_window():
    rows = [
        _mk_row("R1", "2025-06-01", 1, 0.5, 1, 2.0),
        _mk_row("R2", "2026-02-01", 1, 0.5, 1, 2.0),
    ]
    fit, ev = pe.split_window(rows, fit_until="2025-12-31", eval_from="2026-01-01")
    assert {r["race_id"] for r in fit} == {"R1"}
    assert {r["race_id"] for r in ev} == {"R2"}


def test_race_table_excludes_dead_heat_and_no_winner():
    rows = [
        # R1: 正常（勝者 1 頭）。
        _mk_row("R1", "2026-01-10", 1, 0.6, 1, 2.0),
        _mk_row("R1", "2026-01-10", 2, 0.4, 2, 3.0),
        # R2: 同着 1 着 2 頭 → win の R² 母集合から除外。
        _mk_row("R2", "2026-01-10", 1, 0.5, 1, 2.0),
        _mk_row("R2", "2026-01-10", 2, 0.5, 1, 3.0),
        # R3: 勝者行なし（全頭中止）→ 除外。
        _mk_row("R3", "2026-01-10", 1, 0.5, None, 2.0),
        _mk_row("R3", "2026-01-10", 2, 0.5, None, 3.0),
    ]
    table = pe.build_race_table(rows)
    assert sorted(table.races.keys()) == ["R1", "R2", "R3"]
    usable = table.winner_races()
    assert list(usable.keys()) == ["R1"]
    assert table.excluded["dead_heat"] == 1
    assert table.excluded["no_winner"] == 1


def test_winner_prob_extraction_per_system():
    rows = [
        _mk_row("R1", "2026-01-10", 1, 0.6, 1, 2.0, pure=0.3),
        _mk_row("R1", "2026-01-10", 2, 0.4, 2, 3.0, pure=0.7),
    ]
    table = pe.build_race_table(rows)
    races = table.winner_races()
    blended = pe.winner_probs(races, "model_win")
    pure = pe.winner_probs(races, "model_win_pure")
    market = pe.winner_probs_market(races)
    assert blended == [(pytest.approx(0.6), 2)]
    assert pure == [(pytest.approx(0.3), 2)]
    # market: implied [.5, 1/3] → q_winner = .5/(5/6) = 0.6
    assert market == [(pytest.approx(0.6), 2)]


if __name__ == "__main__":
    # 自走式（CI の predict-check ジョブと同じ `python3 <file>` 実行）。
    # 依存が揃っていれば pytest ランナーに委譲する（不在時は冒頭の import ガードで skip 済み）。
    sys.exit(pytest.main([__file__, "-q"]))
