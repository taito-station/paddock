#!/usr/bin/env python3
"""確率系統（pure / blended / market）の校正・識別力を dump TSV から評価する（#703 Phase 1: 計測器）。

入力は `analyze backtest --dump-features` の TSV（pure 3 列を含む 56 列版。列はヘッダ名で引くため
列順・列追加に不変）。出す指標:

  (1) Bolton-Chapman 擬似 R² と対市場 ΔR²（主 KPI・Benter 1994 の評価軸）
      R² = 1 − Σ_races ln p_sys(勝者) / Σ_races ln(1/頭数)。ΔR² はレース単位ブートストラップで 95% CI。
  (2) CORP reliability（PAV = isotonic 回帰・Dimitriadis et al. 2021 PNAS）＋ consistency band
      （校正仮説 y~Bernoulli(p) の再標本化による帯。帯の外に出た点が「有意なズレ」）。
  (3) CORP スコア分解 S̄ = MCB − DSC + UNC（Brier を miscalibration / discrimination /
      uncertainty に分解。PAV ビンなので等幅/等度数ビンの恣意性がない）。
      ※計画時は Ferro-Fricker 2012 のバイアス補正 3 分解を想定したが、PAV を共有できる CORP 分解
        （Triptych, arXiv:2301.10803）の方がビン選択の恣意性ごと消えるためこちらを正とした。
  (4) 忠実性サニティ: blended の win Brier / レース数が `analyze backtest` の出力と一致すること
      （#309 のパターン。dump と評価器の乖離を検知する）。

窓分割: --fit-until / --eval-from（例: fit 2025-01-01〜2025-12-31 / eval 2026-01-01〜）。
λ / (a,b) 等のパラメータ推定は fit 窓のみ、採否判断は eval 窓のみ（in-sample 掃引の再発防止。
窓定義の正本は docs/specifications/backtest.md）。

使い方:
  python3 scripts/predict-check/prob_eval.py bt_dump.tsv \
      --fit-until 2025-12-31 --eval-from 2026-01-01
"""

from __future__ import annotations

import argparse
import sys
from collections import Counter, OrderedDict
from dataclasses import dataclass, field
from datetime import date as _date

import numpy as np

# dump TSV の必須列（ヘッダ名参照）。pure 3 列が無い旧 dump は明示エラーで再生成を促す。
REQUIRED_COLUMNS = [
    "race_id",
    "date",
    "horse_num",
    "model_win",
    "model_place",
    "model_show",
    "model_win_pure",
    "model_place_pure",
    "model_show_pure",
    "finishing_position",
    "win_odds",
    "popularity",
]

_FLOAT_COLS = {
    "model_win",
    "model_place",
    "model_show",
    "model_win_pure",
    "model_place_pure",
    "model_show_pure",
    "win_odds",
}
_INT_COLS = {"horse_num", "finishing_position", "popularity"}

# ln(0) 回避の下限。勝者確率 0 は「モデルが勝者に確率を置かなかった」極端例で、件数は必ず可視化する。
_PROB_FLOOR = 1e-9


def load_dump(path: str) -> list[dict]:
    """dump TSV をヘッダ名参照で読み、行 dict のリストを返す。

    必須列の欠落は ValueError（旧スキーマ dump の取り違えを黙って進めない）。空セルは None。
    列数不一致の行はスキップして件数を stderr に出す（feature_resolution_diag.py と同じ規律）。
    """
    rows: list[dict] = []
    skipped = 0
    with open(path, encoding="utf-8") as f:
        header = f.readline().rstrip("\n").split("\t")
        idx = {name: i for i, name in enumerate(header)}
        missing = [c for c in REQUIRED_COLUMNS if c not in idx]
        if missing:
            raise ValueError(
                f"dump に必須列がありません: {missing}。pure 3 列を含む 56 列版の dump を"
                " 最新 binary（cargo build --release 後の analyze backtest --dump-features）で再生成してください。"
            )
        n_cols = len(header)
        for lineno, line in enumerate(f, start=2):
            cells = line.rstrip("\n").split("\t")
            if len(cells) != n_cols:
                skipped += 1
                continue
            row: dict = {}
            for name in REQUIRED_COLUMNS:
                cell = cells[idx[name]]
                try:
                    if cell == "":
                        row[name] = None
                    elif name in _FLOAT_COLS:
                        v = float(cell)
                        # NaN/inf は「黙って指標を汚染する」最悪の失敗モードなので即エラー
                        # （採否ゲートの計測器として、静かな汚染より停止を選ぶ）。
                        if not np.isfinite(v):
                            raise ValueError(f"非有限値 {cell!r}")
                        row[name] = v
                    elif name in _INT_COLS:
                        row[name] = int(cell)
                    elif name == "date":
                        _date.fromisoformat(cell)  # 形式検証のみ（保持は文字列）
                        row[name] = cell
                    else:
                        row[name] = cell
                except ValueError as e:
                    raise ValueError(
                        f"{path}:{lineno} 列 {name!r} のセル {cell!r} が不正です: {e}"
                    ) from e
            rows.append(row)
    if skipped:
        print(f"警告: 列数不一致の行を {skipped} 件スキップしました（dump の整合性を確認）。", file=sys.stderr)
    return rows


def market_probs(race_rows: list[dict]) -> list[float] | None:
    """レース内全馬の確定単勝オッズから市場含意確率を再構成する。

    q_i = (1/odds_i) / Σ_j (1/odds_j)（オーバーラウンド除去 = estimate.rs の blend と同じ正規化）。
    1 頭でもオッズ欠落・値域違反（<1.0 / 非有限）があるレースは None（市場系統の母集合から除外）。

    注意: dump の `win_odds` は馬単位で「当時 race_odds スナップショット優先・無ければ PDF 確定
    単勝」の fallback（backtest.rs）。スナップショットが部分カバレッジのレースでは時点の異なる
    オッズがレース内に混在した「市場確率」になりうる（既存挙動の継承）。
    """
    implied = []
    for r in race_rows:
        o = r.get("win_odds")
        if o is None or not np.isfinite(o) or o < 1.0:
            return None
        implied.append(1.0 / o)
    total = sum(implied)
    return [v / total for v in implied]


def pav(p: np.ndarray, y: np.ndarray) -> np.ndarray:
    """isotonic 回帰（Pool Adjacent Violators）。p 昇順で y の非減少あてはめを返す（元の行順）。

    CORP reliability（Dimitriadis et al. 2021）の中核。ビン境界をデータから最適決定するため
    等幅/等度数ビンの恣意性がない。返り値は各サンプルの校正済み確率（CEP）。

    同値 p（tie）は PAV の前にひとつのブロックへプールする。プールしないと同じ予測値に
    異なる CEP が割り当てられ、結果が入力の行順に依存してしまう（市場含意確率は同オッズで
    tie が現実に発生する）。
    """
    p = np.asarray(p, dtype=float)
    y = np.asarray(y, dtype=float)
    order = np.argsort(p, kind="stable")
    p_sorted = p[order]
    y_sorted = y[order]
    # tie を事前プール: 同値 p ごとの (総和, 件数) を初期ブロックにする。
    _, group_start = np.unique(p_sorted, return_index=True)
    group_sums = np.add.reduceat(y_sorted, group_start)
    group_counts = np.diff(np.append(group_start, len(p_sorted)))
    # 各ブロックを (総和, 件数) で持ち、平均の逆転（前 > 後）が消えるまで後ろからマージする。
    sums: list[float] = []
    counts: list[int] = []
    for s0, c0 in zip(group_sums, group_counts):
        sums.append(float(s0))
        counts.append(int(c0))
        while len(sums) > 1 and sums[-2] * counts[-1] > sums[-1] * counts[-2]:
            s, c = sums.pop(), counts.pop()
            sums[-1] += s
            counts[-1] += c
    fitted_sorted = np.repeat(
        np.array(sums) / np.array(counts), np.array(counts)
    )
    fitted = np.empty_like(fitted_sorted)
    fitted[order] = fitted_sorted
    return fitted


def corp_decomposition(p: np.ndarray, y: np.ndarray) -> dict:
    """CORP スコア分解: Brier = MCB − DSC + UNC（恒等式）。

    MCB = S̄(p) − S̄(PAV∘p)（miscalibration・0 以上）、
    DSC = UNC − S̄(PAV∘p)（discrimination・大きいほど識別力あり）、
    UNC = ȳ(1−ȳ)（uncertainty・予測に依らない）。
    """
    p = np.asarray(p, dtype=float)
    y = np.asarray(y, dtype=float)
    brier = float(np.mean((p - y) ** 2))
    fitted = pav(p, y)
    s_pav = float(np.mean((fitted - y) ** 2))
    ybar = float(np.mean(y))
    unc = ybar * (1.0 - ybar)
    return {
        "brier": brier,
        "mcb": brier - s_pav,
        "dsc": unc - s_pav,
        "unc": unc,
        "n": len(p),
    }


def pseudo_r2(races: list[tuple[float, int]]) -> float:
    """Bolton-Chapman 擬似 R²。races は (勝者に置いた確率, 頭数) のリスト。

    R² = 1 − Σ ln p(勝者) / Σ ln(1/頭数)。一様予測で 0、完全予測で 1。
    """
    num = 0.0
    den = 0.0
    for p_win, n in races:
        num += np.log(max(p_win, _PROB_FLOOR))
        den += np.log(1.0 / n)
    if den == 0.0:
        # レースが空、または全レース 1 頭立て（ln(1)=0）。0/0 で黙って死なず明示する。
        raise ValueError("擬似 R² の分母が 0 です（対象レースが空か全レース 1 頭立て）")
    return float(1.0 - num / den)


def delta_r2_ci(
    races_a: list[tuple[float, int]],
    races_b: list[tuple[float, int]],
    n_boot: int = 1000,
    seed: int = 42,
) -> tuple[float, float]:
    """ΔR² = R²(A) − R²(B) のレース単位・対応ありブートストラップ 95% CI。

    A/B は同一レース集合を同順で並べたもの（対応が崩れると CI が無意味になるため長さ一致を要求）。
    """
    if len(races_a) != len(races_b):
        raise ValueError("ΔR² のブートストラップは同一レース集合（同長・同順）が前提です")
    rng = np.random.default_rng(seed)
    n = len(races_a)
    deltas = np.empty(n_boot)
    a = np.array(races_a, dtype=float)
    b = np.array(races_b, dtype=float)
    for i in range(n_boot):
        sel = rng.integers(0, n, size=n)
        deltas[i] = pseudo_r2([tuple(x) for x in a[sel]]) - pseudo_r2(
            [tuple(x) for x in b[sel]]
        )
    return float(np.quantile(deltas, 0.025)), float(np.quantile(deltas, 0.975))


@dataclass
class RaceTable:
    """レース単位に整列した dump。excluded は win 指標（R²）の母集合から外した理由別件数。"""

    races: "OrderedDict[str, list[dict]]" = field(default_factory=OrderedDict)
    excluded: Counter = field(default_factory=Counter)

    def winner_races(self) -> "OrderedDict[str, list[dict]]":
        """勝者がちょうど 1 頭のレースのみ（同着 1 着・勝者行なしは除外し件数を記録）。"""
        out: "OrderedDict[str, list[dict]]" = OrderedDict()
        self.excluded.clear()
        for rid, rows in self.races.items():
            winners = [r for r in rows if r.get("finishing_position") == 1]
            if len(winners) == 1:
                out[rid] = rows
            elif len(winners) == 0:
                self.excluded["no_winner"] += 1
            else:
                self.excluded["dead_heat"] += 1
        return out


def build_race_table(rows: list[dict]) -> RaceTable:
    table = RaceTable()
    for r in rows:
        table.races.setdefault(r["race_id"], []).append(r)
    # excluded を先に確定させておく（winner_races は何度呼んでも冪等）。
    table.winner_races()
    return table


def validate_windows(fit_until: str | None, eval_from: str | None) -> str | None:
    """窓引数の検証。エラーなら理由文字列、正常なら None。

    片方だけの指定は「fit 窓に eval 行が混入した重複窓」を黙って出す事故になるため拒否する
    （in-sample 掃引防止という本ツールの目的を裏切る）。日付は ISO 形式のみ受け付ける
    （`2025/12/31` 等は辞書順比較で黙って誤窓になるため）。"""
    if (fit_until is None) != (eval_from is None):
        return "--fit-until と --eval-from は両方指定するか両方省略してください（片方だけは窓が重複します）"
    if fit_until is None:
        return None
    for name, v in (("--fit-until", fit_until), ("--eval-from", eval_from)):
        try:
            _date.fromisoformat(v)
        except ValueError:
            return f"{name} は YYYY-MM-DD 形式で指定してください: {v!r}"
    if fit_until >= eval_from:
        return f"--fit-until ({fit_until}) は --eval-from ({eval_from}) より前の日付にしてください（窓の重複禁止）"
    return None


def split_window(
    rows: list[dict], fit_until: str | None, eval_from: str | None
) -> tuple[list[dict], list[dict]]:
    """date（YYYY-MM-DD 文字列・辞書順=時系列順）で fit/eval に分割する。

    引数は `validate_windows` で検証済みであること（両方指定 or 両方 None）。"""
    fit = [r for r in rows if fit_until is None or r["date"] <= fit_until]
    ev = [r for r in rows if eval_from is not None and r["date"] >= eval_from]
    return fit, ev


def winner_probs(
    races: "OrderedDict[str, list[dict]]", key: str
) -> list[tuple[float, int]]:
    """各レースの (勝者に置いた確率, 頭数)。key は model_win / model_win_pure。"""
    out = []
    for rows in races.values():
        winner = next(r for r in rows if r.get("finishing_position") == 1)
        out.append((float(winner[key]), len(rows)))
    return out


def winner_probs_market(
    races: "OrderedDict[str, list[dict]]",
) -> list[tuple[float, int]]:
    """市場系統の (勝者確率, 頭数)。オッズ不完全なレースは飛ばす（呼び出し側で母集合を揃えること）。"""
    out = []
    for rows in races.values():
        q = market_probs(rows)
        if q is None:
            continue
        widx = next(i for i, r in enumerate(rows) if r.get("finishing_position") == 1)
        out.append((q[widx], len(rows)))
    return out


# ---------- レポート ----------

_TARGETS = {"win": 1, "place": 2, "show": 3}
_SYSTEMS = ["market", "pure", "blended"]


def _target_arrays(
    races: "OrderedDict[str, list[dict]]", target: str, system: str
) -> tuple[np.ndarray, np.ndarray] | None:
    """全出走馬の (予測 p, 実現 y) を系統×ターゲットで並べる。market は place/show を持たない。"""
    thresh = _TARGETS[target]
    ps: list[float] = []
    ys: list[float] = []
    for rows in races.values():
        if system == "market":
            if target != "win":
                return None
            q = market_probs(rows)
            if q is None:
                continue
            for r, qi in zip(rows, q):
                ps.append(qi)
                ys.append(_label(r, thresh))
        else:
            key = f"model_{target}" + ("_pure" if system == "pure" else "")
            for r in rows:
                ps.append(float(r[key]))
                ys.append(_label(r, thresh))
    if not ps:
        return None
    return np.array(ps), np.array(ys)


def _label(row: dict, thresh: int) -> float:
    """着順ラベル。着順なし（中止・除外等）は不的中として 0（evaluate.rs と同じ扱い）。"""
    fin = row.get("finishing_position")
    return 1.0 if fin is not None and fin <= thresh else 0.0


def reliability_band(
    p: np.ndarray, y: np.ndarray, n_boot: int = 1000, seed: int = 42, n_grid: int = 10
) -> list[dict]:
    """CORP reliability の consistency band。

    校正仮説 H0: y ~ Bernoulli(p) の下で y を再標本化 → PAV を再フィット → p の分位グリッド上の
    CEP の 5%/95% 帯（**90% pointwise**）を取る。

    読み方の注意 2 点:
    - 90% pointwise なので、H0（完全校正）の下でも 10 点グリッドで**約 1 点は帯外に出うる**
      （多重性未調整）。1〜2/10 の帯外を有意なズレと読まない。系統的なズレの判定は
      「大半の点が同方向に帯外」（例: 9〜10/10）で行う。
    - H0 の再標本は独立 Bernoulli で、レース内の従属（win は 1 レース 1 勝者の負相関）を
      無視している。従属を入れた場合より帯は広め（保守側）に出ると考えられる。
    """
    rng = np.random.default_rng(seed)
    order = np.argsort(p, kind="stable")
    p_sorted = p[order]
    fitted = pav(p, y)[order]
    n = len(p)
    grid_idx = [min(n - 1, int(q * (n - 1))) for q in np.linspace(0.05, 0.95, n_grid)]
    sims = np.empty((n_boot, n_grid))
    for b in range(n_boot):
        y_sim = (rng.uniform(size=n) < p).astype(float)
        fit_sim = pav(p, y_sim)[order]
        sims[b] = fit_sim[grid_idx]
    lo = np.quantile(sims, 0.05, axis=0)
    hi = np.quantile(sims, 0.95, axis=0)
    out = []
    for g, i in enumerate(grid_idx):
        out.append(
            {
                "p": float(p_sorted[i]),
                "cep": float(fitted[i]),
                "band_lo": float(lo[g]),
                "band_hi": float(hi[g]),
                "outside": bool(fitted[i] < lo[g] or fitted[i] > hi[g]),
            }
        )
    return out


def fidelity(rows: list[dict]) -> dict:
    """忠実性サニティ: blended の win Brier（全出走馬）とレース数。analyze backtest の出力と突合する。"""
    p = np.array([float(r["model_win"]) for r in rows])
    y = np.array([_label(r, 1) for r in rows])
    return {
        "win_brier": float(np.mean((p - y) ** 2)),
        "races": len({r["race_id"] for r in rows}),
        "horses": len(rows),
    }


def _report_window(name: str, rows: list[dict], n_boot: int, seed: int) -> None:
    if not rows:
        print(f"\n== {name}: 対象行なし ==")
        return
    table = build_race_table(rows)
    usable = table.winner_races()
    # 市場系統が使えるレースに母集合を揃える（3 系統の R² を同一レース集合で比較する）。
    market_ok = OrderedDict(
        (rid, rr) for rid, rr in usable.items() if market_probs(rr) is not None
    )
    dates = sorted(r["date"] for r in rows)
    print(f"\n== {name}（{dates[0]}〜{dates[-1]}） ==")
    print(
        f"レース {len(table.races)}（勝者一意 {len(usable)} / 同着除外 {table.excluded['dead_heat']}"
        f" / 勝者なし除外 {table.excluded['no_winner']} / オッズ不完全除外 {len(usable) - len(market_ok)}）"
        f"・出走馬 {len(rows)}"
    )

    fid = fidelity(rows)
    print(
        f"忠実性サニティ: blended win Brier = {fid['win_brier']:.6f}"
        f"（analyze backtest の単勝 Brier と一致すること）・レース {fid['races']}・馬 {fid['horses']}"
    )
    if not market_ok:
        print("市場系統が使えるレースが 0 件のため、この窓の R²/CORP はスキップします。")
        return

    # (1) 擬似 R² と ΔR²（母集合 = market_ok）。
    wp = {
        "market": winner_probs_market(market_ok),
        "pure": winner_probs(market_ok, "model_win_pure"),
        "blended": winner_probs(market_ok, "model_win"),
    }
    floored = sum(1 for sys_races in wp.values() for p_, _ in sys_races if p_ <= _PROB_FLOOR)
    if floored:
        print(f"警告: 勝者確率 ≤ {_PROB_FLOOR} を {floored} 件 floor しました。")
    r2 = {s: pseudo_r2(wp[s]) for s in _SYSTEMS}
    print(f"擬似 R²（n={len(wp['market'])}R）: " + "  ".join(f"{s}={r2[s]:.4f}" for s in _SYSTEMS))
    for s in ["pure", "blended"]:
        lo, hi = delta_r2_ci(wp[s], wp["market"], n_boot=n_boot, seed=seed)
        print(
            f"ΔR²({s} − market) = {r2[s] - r2['market']:+.4f}  [95% CI {lo:+.4f}, {hi:+.4f}]"
        )

    # (2)(3) CORP 分解と consistency band（win は 3 系統、place/show は pure/blended のみ）。
    print("CORP 分解（Brier = MCB − DSC + UNC）:")
    print(f"  {'target':6} {'system':8} {'n':>7} {'Brier':>9} {'MCB':>9} {'DSC':>9} {'UNC':>9} 帯外点")
    for target in _TARGETS:
        for system in _SYSTEMS:
            arrays = _target_arrays(market_ok, target, system)
            if arrays is None:
                continue
            p_, y_ = arrays
            d = corp_decomposition(p_, y_)
            # band の再標本は PAV 再フィットが重いため上限 400 回にキャップする
            # （--bootstrap は ΔR² の CI にのみフルに効く）。
            band = reliability_band(p_, y_, n_boot=min(n_boot, 400), seed=seed)
            outside = sum(1 for b in band if b["outside"])
            print(
                f"  {target:6} {system:8} {d['n']:>7} {d['brier']:>9.5f} {d['mcb']:>9.5f}"
                f" {d['dsc']:>9.5f} {d['unc']:>9.5f} {outside}/{len(band)}"
            )


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dump", help="analyze backtest --dump-features の TSV（pure 3 列を含む 56 列版）")
    ap.add_argument("--fit-until", default=None, help="fit 窓の終端日（YYYY-MM-DD・含む。--eval-from と必ず対で指定）")
    ap.add_argument("--eval-from", default=None, help="eval 窓の開始日（YYYY-MM-DD・含む。--fit-until と必ず対で指定）")
    ap.add_argument(
        "--bootstrap",
        type=int,
        default=1000,
        help="ブートストラップ回数（既定 1000。ΔR² の CI に適用。consistency band は最大 400 回にキャップ）",
    )
    ap.add_argument("--seed", type=int, default=42)
    args = ap.parse_args()

    err = validate_windows(args.fit_until, args.eval_from)
    if err:
        ap.error(err)
    rows = load_dump(args.dump)
    print(f"読み込み: {len(rows)} 行（{args.dump}）")
    if args.fit_until is None and args.eval_from is None:
        _report_window("全期間", rows, args.bootstrap, args.seed)
        return
    fit, ev = split_window(rows, args.fit_until, args.eval_from)
    _report_window(f"fit 窓（〜{args.fit_until}）", fit, args.bootstrap, args.seed)
    _report_window(f"eval 窓（{args.eval_from}〜）", ev, args.bootstrap, args.seed)


if __name__ == "__main__":
    main()
