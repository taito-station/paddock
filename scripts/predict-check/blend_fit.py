#!/usr/bin/env python3
"""対数プール（Benter 型）の指数 (a,b) を dump TSV から MLE 推定し、eval 窓で線形ブレンドと
比較する（#703 Phase 3）。

モデル: レース内の勝者尤度を P(win=i) ∝ u_i^A · q_i^B とする条件付きロジット。
  - u_i = dump の `model_win_pure`（純モデル・win_power γ=1.25 適用後）
  - q_i = 確定単勝オッズから再構成した市場含意確率（オーバーラウンド除去）
対数尤度は (A,B) について凹（条件付きロジット）なので Newton 法で一意に収束する。

**パイプラインとの対応（冪の可換性）**: 本番パイプラインは
`estimate → LogPool(a,b) → win_power γ` の順で、最終 win ∝ p_model^{aγ}·q^{bγ} = u^{a}·q^{bγ}。
よって本スクリプトの (Â, B̂) は
  - `--win-power 1.25` 併用時: `--log-pool-a Â --log-pool-b B̂/1.25`
  - win_power なし: `--log-pool-a Â×1.25 --log-pool-b B̂`
で CLI に渡すと同じ合成確率になる（正規化により厳密に等価）。

採用ゲート（backtest.md 評価プロトコル・決定ログ #703）:
  fit 窓で (Â,B̂) を推定 → eval 窓で ΔR²(logpool − market) をレース単位 bootstrap 95% CI 付きで
  算出し、CI が 0 を上回る（点推定の目安 +0.003 以上）ことが主判定。参考として production
  線形ブレンド（dump の `model_win` 列）の ΔR² も並記する。

使い方:
  python3 scripts/predict-check/blend_fit.py bt_dump.tsv \
      --fit-until 2025-12-31 --eval-from 2026-01-01
"""

from __future__ import annotations

import argparse
from collections import OrderedDict

import numpy as np

import prob_eval as pe


def race_features(rows: list[dict]) -> tuple[np.ndarray, np.ndarray, int] | None:
    """1 レースの (u, q, 勝者 index) を返す共通ヘルパ。母集合条件を **単一箇所** で定義する
    （3 系統の R² 比較は同一母集合が前提のため、フィルタの重複実装は禁物）。

    条件: 全馬にオッズ（market_probs が非 None）・全馬 u>0（ln が要るため。u=0 × A>0 は
    当該馬の確率 0 と等価だが、勝者が u=0 のとき logL が発散する）。非有限 u は load_dump が
    読み込み時に止める設計だが、直呼び経路のため防御的に除外条件へ含める。
    勝者一意は呼び出し側（winner_races）が保証する。"""
    q = pe.market_probs(rows)
    if q is None:
        return None
    u = np.array([float(r["model_win_pure"]) for r in rows])
    if not np.all(np.isfinite(u)) or np.any(u <= 0.0):
        return None
    widx = next(i for i, r in enumerate(rows) if r.get("finishing_position") == 1)
    return u, np.array(q), widx


def logpool_races(
    races: "OrderedDict[str, list[dict]]",
) -> tuple[list[tuple[np.ndarray, np.ndarray, int]], dict]:
    """(x1=ln u, x2=ln q, 勝者 index) をレースごとに並べる（母集合は race_features が定義）。"""
    out = []
    excluded = {"zero_model": 0, "no_market": 0}
    for rows in races.values():
        feat = race_features(rows)
        if feat is None:
            # 除外理由の内訳（レポート用）。判定は race_features と同一条件。
            if pe.market_probs(rows) is None:
                excluded["no_market"] += 1
            else:
                excluded["zero_model"] += 1
            continue
        u, q, widx = feat
        out.append((np.log(u), np.log(q), widx))
    return out, excluded


def loglik(races, theta: np.ndarray) -> float:
    total = 0.0
    for x1, x2, w in races:
        z = theta[0] * x1 + theta[1] * x2
        z -= z.max()  # 数値安定化（logsumexp）
        total += float(z[w] - np.log(np.exp(z).sum()))
    return total


def fit_ab(races, max_iter: int = 50) -> tuple[np.ndarray, np.ndarray, float]:
    """Newton 法で (A,B) を MLE 推定。返り値: (θ̂, 標準誤差, logL(θ̂))。

    条件付きロジットの logL は凹で、Hessian = −Σ Cov_p[x]（半負定値）。SE は観測情報行列の
    逆行列の対角から取る。
    """
    theta = np.array([0.2, 1.0])  # 線形 α=0.2 の感覚に近い初期値（凹なので収束は初期値非依存）
    converged = False
    hess = np.zeros((2, 2))
    for _ in range(max_iter):
        grad = np.zeros(2)
        hess = np.zeros((2, 2))
        for x1, x2, w in races:
            x = np.stack([x1, x2])  # (2, n)
            z = theta @ x
            z -= z.max()
            p = np.exp(z)
            p /= p.sum()
            mean = x @ p
            grad += x[:, w] - mean
            centered = x - mean[:, None]
            hess -= (centered * p) @ centered.T
        # 縮退（全レースで ln u と ln q が共線等）で特異になりうるため ridge 正則化で可逆化
        step = np.linalg.solve(hess - 1e-9 * np.eye(2), grad)
        theta = theta - step
        if np.abs(step).max() < 1e-10:
            converged = True
            break
    if not converged:
        print(f"警告: Newton 法が {max_iter} 反復で未収束（θ̂・SE は参考値として扱うこと）。")
    # 観測情報 = -hess（最終反復の値を再利用）。縮退データでは特異になりうるので明示エラー。
    info = -hess
    try:
        se = np.sqrt(np.diag(np.linalg.inv(info)))
    except np.linalg.LinAlgError as e:
        raise SystemExit(
            "観測情報行列が特異です（ln u と ln q がほぼ共線などのデータ縮退）。母集合を確認してください。"
        ) from e
    return theta, se, loglik(races, theta)


def composite_winner_probs(
    races: "OrderedDict[str, list[dict]]", theta: np.ndarray
) -> list[tuple[float, int]]:
    """(勝者に置いた合成確率, 頭数) のリスト（pseudo_r2 / delta_r2_ci 互換）。

    母集合は logpool_races と同一条件（勝者一意・全馬オッズ・全馬 u>0）に揃えること。
    """
    out = []
    for rows in races.values():
        feat = race_features(rows)
        if feat is None:
            continue
        u, q, widx = feat
        z = theta[0] * np.log(u) + theta[1] * np.log(q)
        z -= z.max()
        p = np.exp(z)
        p /= p.sum()
        out.append((float(p[widx]), len(rows)))
    return out


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("dump", help="analyze backtest --dump-features の TSV（56 列版）")
    ap.add_argument("--fit-until", required=True, help="fit 窓の終端日（YYYY-MM-DD・含む）")
    ap.add_argument("--eval-from", required=True, help="eval 窓の開始日（YYYY-MM-DD・含む）")
    ap.add_argument("--bootstrap", type=int, default=1000, help="ΔR² CI のブートストラップ回数")
    ap.add_argument("--seed", type=int, default=42)
    args = ap.parse_args()

    err = pe.validate_windows(args.fit_until, args.eval_from)
    if err:
        ap.error(err)

    rows = pe.load_dump(args.dump)
    fit_rows, eval_rows = pe.split_window(rows, args.fit_until, args.eval_from)

    windows = {}
    for name, wrows in (("fit", fit_rows), ("eval", eval_rows)):
        table = pe.build_race_table(wrows)
        usable = table.winner_races()
        races, excluded = logpool_races(usable)
        if not races:
            ap.error(f"{name} 窓に対数プール推定可能なレースが 0 件です")
        print(
            f"{name} 窓: 勝者一意 {len(usable)}R → 推定母集合 {len(races)}R"
            f"（オッズ不完全除外 {excluded['no_market']} / u=0 含み除外 {excluded['zero_model']}）"
        )
        windows[name] = (usable, races)

    # (A,B) の MLE（fit 窓のみ）。
    _, fit_races = windows["fit"]
    theta, se, ll = fit_ab(fit_races)
    ll_market = loglik(fit_races, np.array([0.0, 1.0]))
    print("\n-- (A,B) の MLE（fit 窓・合成スケール: P ∝ u^A · q^B） --")
    print(f"  Â = {theta[0]:.4f} ± {1.96 * se[0]:.4f}  /  B̂ = {theta[1]:.4f} ± {1.96 * se[1]:.4f}")
    print(f"  logL 改善（対 市場のみ (0,1)）= {ll - ll_market:+.1f}")
    gamma = 1.25
    if theta[0] < 0.0:
        print(
            "  注意: Â < 0（モデルの逆張り方向）。CLI（--log-pool-a）は 0 <= a <= 10 のみ受理する"
            "ため下記換算は実行不可。負の指数に実質的な意味は無く（最適解が市場のみ側）、"
            "backtest 再現には a=0 を使う。"
        )
    print("  CLI 換算（合成が厳密等価になる指定・win_power γ=1.25 前提）:")
    print(f"    --win-power 1.25 併用時: --log-pool-a {theta[0]:.4f} --log-pool-b {theta[1] / gamma:.4f}")
    print(f"    win_power なし:          --log-pool-a {theta[0] * gamma:.4f} --log-pool-b {theta[1]:.4f}")

    # eval 窓の採用ゲート判定（母集合を合成確率が定義できるレースに揃える）。
    eval_usable, _ = windows["eval"]
    lp = composite_winner_probs(eval_usable, theta)
    market = composite_winner_probs(eval_usable, np.array([0.0, 1.0]))
    # production 線形ブレンド（dump の model_win 列）を同一母集合で参考出力。
    linear = []
    for rows_ in eval_usable.values():
        feat = race_features(rows_)
        if feat is None:
            continue
        _, _, widx = feat
        linear.append((float(rows_[widx]["model_win"]), len(rows_)))

    r2_lp = pe.pseudo_r2(lp)
    r2_mkt = pe.pseudo_r2(market)
    r2_lin = pe.pseudo_r2(linear)
    lo, hi = pe.delta_r2_ci(lp, market, n_boot=args.bootstrap, seed=args.seed)
    lo_lin, hi_lin = pe.delta_r2_ci(linear, market, n_boot=args.bootstrap, seed=args.seed)
    print(f"\n-- eval 窓の採用ゲート（n={len(lp)}R・母集合は 3 系統で同一） --")
    print(f"  R²: market={r2_mkt:.4f}  logpool={r2_lp:.4f}  linear(現行 dump 列)={r2_lin:.4f}")
    print(f"  ΔR²(logpool − market) = {r2_lp - r2_mkt:+.4f}  [95% CI {lo:+.4f}, {hi:+.4f}]")
    print(f"  ΔR²(linear  − market) = {r2_lin - r2_mkt:+.4f}  [95% CI {lo_lin:+.4f}, {hi_lin:+.4f}]")
    print("  主判定: ΔR²(logpool − market) の CI が 0 を上回る（点推定の目安 +0.003 以上）こと。")


if __name__ == "__main__":
    main()
