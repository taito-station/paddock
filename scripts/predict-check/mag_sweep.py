#!/usr/bin/env python3
"""(m, α, γ) の確定チューニング掃引（#284）。dump 3 本（m 焼き込み）× α×γ のオフライン厳密再構成。

再構成の数理（#703 Phase 3 で確立した手法の線形版）:
  dump の pure 列 u ∝ est^γ0（γ0 = dump 生成時の win_power = 1.25）なので est ∝ u^(1/γ0)。
  任意の (α, γ) の最終 win は本番パイプライン（estimate → 線形 blend → win_power）を
  そのまま写して
    ê      = normalize(u^(1/γ0))
    mix_i  = α·ê_i + (1−α)·q_i   （オッズあり馬。q = implied/overround）
             ê_i                  （オッズ欠落馬 = model 値据置・blend_with_market_win と同じ）
    final  = normalize( normalize(mix)^γ )
  で厳密に再現できる（正規化不変性）。忠実性アンカー: (α=0.2, γ=1.25) の再構成が dump の
  `model_win` 列と全行一致（max|Δ| < 1e-9）すること。

評価プロトコル（backtest.md #703・採用ゲートは issue #284 コメントで事前コミット済み）:
  - fit 窓（2025 年）: 格子 21 点 × m 3 本の ΔR²(候補−market) 点推定で候補選択。
    同率（差 < 0.0005）は現行 (m=10, α=0.2, γ=1.25) に α → γ → m の辞書順で近い方。
  - eval 窓（2026-01〜08）:
    (a) paired ΔR²(候補−現行) の 95% CI 下限 > 0
    (b) ΔR²(候補−market) の 95% CI 下限 ≥ −0.001（候補が market 恒等〔α=0 かつ γ=1.0〕なら自動成立）
    (c) win Brier が現行比 +0.0002 以内
  - 母集合: 勝者一意 かつ 全馬オッズあり（market が定義できるレース）。3 系統・全候補で同一。
  - 測定の限界: q はクローズオッズ基準。live オッズでの α 再校正は #218 の管轄。

使い方:
  python3 scripts/predict-check/mag_sweep.py \
      m5=bt_dump_m5.tsv m10=bt_dump_m10.tsv m20=bt_dump_m20.tsv \
      --fit-until 2025-12-31 --eval-from 2026-01-01
"""

from __future__ import annotations

import argparse
from collections import OrderedDict

import numpy as np

import prob_eval as pe

GAMMA0 = 1.25  # dump 生成時の win_power（production 等価フラグ）
DUMP_ALPHA = 0.2  # dump 生成時の blend α（忠実性アンカーの再構成に使う。CURRENT とは別概念）
ALPHAS = [0.0, 0.05, 0.1, 0.2, 0.3, 0.5, 1.0]
GAMMAS = [1.0, 1.25, 1.5]
CURRENT = ("m10", 0.2, 1.25)
TIE_EPS = 0.0005


def reconstruct_win(rows: list[dict], alpha: float, gamma: float) -> np.ndarray:
    """1 レースの最終 win 確率を dump 行から再構成する（本番パイプラインの厳密再現）。

    オッズ欠落馬は model 値据置（blend_with_market_win と同一分岐）。u=0 の馬は est=0 の
    まま流れる（本番と同じ）。
    """
    u = np.array([float(r["model_win_pure"]) for r in rows])
    est = np.power(np.maximum(u, 0.0), 1.0 / GAMMA0)
    total_est = est.sum()
    e_hat = est / total_est if total_est > 0 else est
    # 市場 implied（オッズあり馬のみが母数・blend_with_market_win と同じフィルタ）。
    # 番兵ガード（odds_guard.py）は通さない: netkeiba 番兵に単勝は無く（ADR 0086/0088）、
    # dump の win_odds は Rust backtest が OddsValue ガードを通した値のため。
    implied = np.array(
        [
            1.0 / float(r["win_odds"])
            if r.get("win_odds") is not None
            and np.isfinite(float(r["win_odds"]))
            and float(r["win_odds"]) >= 1.0
            else np.nan
            for r in rows
        ]
    )
    overround = np.nansum(implied)
    if alpha >= 1.0 or not np.any(np.isfinite(implied)) or overround <= 0.0:
        mixed = e_hat.copy()  # no-op ブレンド（α>=1.0 / オッズ皆無）
    else:
        q = implied / overround
        mixed = np.where(np.isfinite(q), alpha * e_hat + (1.0 - alpha) * q, e_hat)
        t = mixed.sum()
        if t > 0:
            mixed = np.minimum(mixed / t, 1.0)
    # win_power（γ=1.0 ちょうどは本番も no-op）。
    if abs(gamma - 1.0) < np.finfo(float).eps:
        final = mixed
    else:
        powered = np.power(mixed, gamma)
        tp = powered.sum()
        final = np.minimum(powered / tp, 1.0) if tp > 0 else powered
    return final


def fidelity_split(
    races: "OrderedDict[str, list[dict]]",
) -> tuple["OrderedDict[str, list[dict]]", list[str], float]:
    """(α=0.2, γ=1.25) の再構成が dump の model_win 列と一致するレースと、しないレースを分ける。

    不一致レースは「dump の win_odds 列から blend 入力を復元できない」既知の乖離クラス:
    race_odds の win 行がレース後に再取得され、as-of フィルタを通る行が取消馬等の断片しか
    ない場合、blend は実質 no-op（blended==pure）になる一方、dump の win_odds は馬単位
    fallback で PDF 確定オッズを示す（実例: 2026-2-kokura-8-7R）。このクラスは再構成不能
    なので母集合から除外する（除外は市場系統にも同じ集合を適用＝比較の公平性を保つ）。
    判定は dump ごとに独立に行い、除外後のレース集合が dump 間で食い違わないことは
    main のレース集合一致検査が停止条件として担保する。
    返り値: (一致レース, 不一致 race_id リスト, 一致レース内の最大絶対差)。
    """
    ok: "OrderedDict[str, list[dict]]" = OrderedDict()
    excluded: list[str] = []
    worst_ok = 0.0
    for rid, rows in races.items():
        recon = reconstruct_win(rows, DUMP_ALPHA, GAMMA0)
        ref = np.array([float(r["model_win"]) for r in rows])
        d = float(np.max(np.abs(recon - ref)))
        if d < 1e-9:
            ok[rid] = rows
            worst_ok = max(worst_ok, d)
        else:
            excluded.append(rid)
    return ok, excluded, worst_ok


def eligible_races(
    races: "OrderedDict[str, list[dict]]",
) -> "OrderedDict[str, list[dict]]":
    """R²/Brier/top1 の母集合: 勝者一意（呼び出し側で保証）かつ全馬オッズあり。"""
    out: "OrderedDict[str, list[dict]]" = OrderedDict()
    for rid, rows in races.items():
        if pe.market_probs(rows) is not None:
            out[rid] = rows
    return out


def candidate_metrics(
    races: "OrderedDict[str, list[dict]]", alpha: float, gamma: float
) -> dict:
    """(勝者確率リスト, win Brier, top1, CORP 用フラット配列) を候補 1 点について算出する。"""
    winner_probs: list[tuple[float, int]] = []
    prob_chunks: list[np.ndarray] = []
    y_chunks: list[np.ndarray] = []
    top1_hits = 0
    for rows in races.values():
        final = reconstruct_win(rows, alpha, gamma)
        widx = next(i for i, r in enumerate(rows) if r.get("finishing_position") == 1)
        winner_probs.append((float(final[widx]), len(rows)))
        y = np.array([1.0 if r.get("finishing_position") == 1 else 0.0 for r in rows])
        prob_chunks.append(final)
        y_chunks.append(y)
        if int(np.argmax(final)) == widx:
            top1_hits += 1
    probs_all = np.concatenate(prob_chunks)
    y_all = np.concatenate(y_chunks)
    return {
        "winner_probs": winner_probs,
        "brier": float(np.mean((probs_all - y_all) ** 2)),
        "top1": top1_hits / len(races),
        "probs_all": probs_all,
        "y_all": y_all,
    }


def distance_to_current(label: str, alpha: float, gamma: float) -> tuple[float, float, float]:
    """同率タイブレーク用の現行 (m10, 0.2, 1.25) からの辞書順距離（α → γ → m）。"""
    m_dist = 0.0 if label == CURRENT[0] else 1.0
    return (abs(alpha - CURRENT[1]), abs(gamma - CURRENT[2]), m_dist)


def is_market_identical(alpha: float, gamma: float) -> bool:
    """再構成が市場確率 q に恒等となる条件（α=0 かつ γ=1.0）。

    α=0 でも γ≠1.0 は normalize(q^γ) ≠ q なので恒等ではない（ゲート (b) を
    スキップしてはならない）。事前コミット文言（issue #284 コメント）の「α=0」には
    γ 条件が欠けており、本実装で補正した（決定ログに注記あり）。
    """
    return alpha == 0.0 and abs(gamma - 1.0) < np.finfo(float).eps


def judge_gates(
    lo_a: float, lo_b: float | None, brier_cand: float, brier_cur: float
) -> tuple[bool, bool, bool, bool]:
    """採用ゲート (a)(b)(c) の判定（事前コミット: issue #284 コメント）。

    lo_b=None は候補が market 恒等（ΔR²(候補−market) ≡ 0）で (b) 自動成立。
    返り値: (gate_a, gate_b, gate_c, 総合判定)。
    """
    gate_a = lo_a > 0
    gate_b = True if lo_b is None else lo_b >= -0.001
    gate_c = brier_cand <= brier_cur + 0.0002
    return gate_a, gate_b, gate_c, gate_a and gate_b and gate_c


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "dumps",
        nargs=3,
        help="label=path の 3 指定（例: m5=bt_dump_m5.tsv m10=... m20=...）",
    )
    ap.add_argument("--fit-until", required=True)
    ap.add_argument("--eval-from", required=True)
    ap.add_argument("--bootstrap", type=int, default=1000)
    ap.add_argument("--seed", type=int, default=42)
    args = ap.parse_args()

    err = pe.validate_windows(args.fit_until, args.eval_from)
    if err:
        ap.error(err)

    # CLI 検証: label=path 形式・ラベル一意・現行ラベルの存在（掃引完走後の KeyError や
    # distance_to_current の黙った歪みを起動時に止める）。
    labels: list[str] = []
    for spec in args.dumps:
        if "=" not in spec:
            ap.error(f"label=path 形式で指定してください: {spec}")
        labels.append(spec.split("=", 1)[0])
    if len(set(labels)) != len(labels):
        ap.error(f"dump ラベルが重複しています: {labels}")
    if CURRENT[0] not in labels:
        ap.error(f"現行ラベル {CURRENT[0]} が dump 指定にありません: {labels}")

    windows: dict[str, dict[str, "OrderedDict[str, list[dict]]"]] = {"fit": {}, "eval": {}}
    raw_counts: dict[str, int] = {}
    for spec in args.dumps:
        label, path = spec.split("=", 1)
        rows = pe.load_dump(path)
        raw_counts[label] = len(rows)
        fit_rows, eval_rows = pe.split_window(rows, args.fit_until, args.eval_from)
        for wname, wrows in (("fit", fit_rows), ("eval", eval_rows)):
            table = pe.build_race_table(wrows)
            usable = eligible_races(table.winner_races())
            if not usable:
                ap.error(f"{label}/{wname} 窓の母集合が 0 件です")
            ok, excluded, worst = fidelity_split(usable)
            frac = len(excluded) / len(usable)
            print(
                f"忠実性アンカー[{label}/{wname}]: max|Δ|(一致分) = {worst:.3e}"
                f" / 再構成不能で除外 {len(excluded)}R ({frac:.1%})"
            )
            if frac > 0.01:
                raise SystemExit(
                    f"{label}/{wname}: 再構成不能レースが 1% を超過（{len(excluded)}/{len(usable)}）。"
                    " dump の生成フラグ・SHA を確認（既知の乖離クラスなら少数のはず）"
                )
            windows[wname][label] = ok

    # 行数一致確認（issue #284 成功基準: 3 本の行数・レース集合が一致）。
    if len(set(raw_counts.values())) != 1:
        raise SystemExit(f"dump の行数が不一致（SHA を揃えて再生成）: {raw_counts}")

    # 母集合の一致確認（3 dump は同一レース集合のはず）。
    for wname in ("fit", "eval"):
        keysets = {label: tuple(r.keys()) for label, r in windows[wname].items()}
        base = next(iter(keysets.values()))
        if not all(k == base for k in keysets.values()):
            raise SystemExit(
                f"{wname} 窓のレース集合が dump 間で不一致。SHA を揃えて再生成"
                "（fidelity 除外集合が dump 間で食い違った可能性もある。各 dump の除外 R 数を突合）"
            )
    n_fit = len(next(iter(windows["fit"].values())))
    n_eval = len(next(iter(windows["eval"].values())))
    print(f"母集合: fit {n_fit}R / eval {n_eval}R（勝者一意・全馬オッズあり・3 dump 共通）")

    # market 系統（どの dump でも同一。fit の代表 dump から）。
    rep = next(iter(windows["fit"]))
    market_fit = pe.winner_probs_market(windows["fit"][rep])
    market_eval = pe.winner_probs_market(windows["eval"][rep])
    r2_market_fit = pe.pseudo_r2(market_fit)
    r2_market_eval = pe.pseudo_r2(market_eval)

    # fit 窓の格子評価。
    print(f"\n-- fit 窓（R²(market)={r2_market_fit:.4f}） --")
    print(f"  {'m':>4} {'α':>5} {'γ':>5} {'ΔR²(−mkt)':>10} {'Brier':>9} {'top1':>7}")
    results = []
    for label in windows["fit"]:
        for alpha in ALPHAS:
            for gamma in GAMMAS:
                met = candidate_metrics(windows["fit"][label], alpha, gamma)
                dr2 = pe.pseudo_r2(met["winner_probs"]) - r2_market_fit
                results.append((label, alpha, gamma, dr2, met))
                print(
                    f"  {label:>4} {alpha:>5.2f} {gamma:>5.2f} {dr2:>+10.4f}"
                    f" {met['brier']:>9.5f} {met['top1']:>7.3f}"
                )

    # 候補選択（事前コミット: ΔR² 最大・同率は現行に近い方）。
    best_dr2 = max(r[3] for r in results)
    tied = [r for r in results if best_dr2 - r[3] < TIE_EPS]
    tied.sort(key=lambda r: distance_to_current(r[0], r[1], r[2]))
    cand_label, cand_alpha, cand_gamma, cand_fit_dr2, _ = tied[0]
    print(
        f"\n候補: ({cand_label}, α={cand_alpha}, γ={cand_gamma})"
        f"  fit ΔR²={cand_fit_dr2:+.4f}（同率 {len(tied)} 点・現行距離で選択）"
    )

    # eval 窓の採用ゲート。
    cur_label, cur_alpha, cur_gamma = CURRENT
    cand_eval = candidate_metrics(windows["eval"][cand_label], cand_alpha, cand_gamma)
    cur_eval = candidate_metrics(windows["eval"][cur_label], cur_alpha, cur_gamma)
    r2_cand = pe.pseudo_r2(cand_eval["winner_probs"])
    r2_cur = pe.pseudo_r2(cur_eval["winner_probs"])

    print(f"\n-- eval 窓の採用ゲート（R²(market)={r2_market_eval:.4f}） --")
    lo_a, hi_a = pe.delta_r2_ci(
        cand_eval["winner_probs"], cur_eval["winner_probs"], n_boot=args.bootstrap, seed=args.seed
    )
    if is_market_identical(cand_alpha, cand_gamma):
        lo_b: float | None = None
    else:
        lo_b, hi_b = pe.delta_r2_ci(
            cand_eval["winner_probs"], market_eval, n_boot=args.bootstrap, seed=args.seed
        )
    gate_a, gate_b, gate_c, verdict = judge_gates(
        lo_a, lo_b, cand_eval["brier"], cur_eval["brier"]
    )
    print(
        f"  (a) paired ΔR²(候補−現行) = {r2_cand - r2_cur:+.4f}"
        f" [95% CI {lo_a:+.4f}, {hi_a:+.4f}] → {'PASS' if gate_a else 'FAIL'}"
    )
    if lo_b is None:
        print("  (b) 候補は market 恒等（α=0 かつ γ=1.0）→ ΔR²(候補−market) ≡ 0 で自動成立 → PASS")
    else:
        print(
            f"  (b) ΔR²(候補−market) = {r2_cand - r2_market_eval:+.4f}"
            f" [95% CI {lo_b:+.4f}, {hi_b:+.4f}] → {'PASS' if gate_b else 'FAIL'}"
        )
    print(
        f"  (c) win Brier 候補 {cand_eval['brier']:.5f} vs 現行 {cur_eval['brier']:.5f}"
        f"（許容 +0.0002）→ {'PASS' if gate_c else 'FAIL'}"
    )
    corp_cand = pe.corp_decomposition(cand_eval["probs_all"], cand_eval["y_all"])
    corp_cur = pe.corp_decomposition(cur_eval["probs_all"], cur_eval["y_all"])
    print(
        f"  参考: top1 候補 {cand_eval['top1']:.3f} vs 現行 {cur_eval['top1']:.3f}"
        f" / eval ΔR²(現行−market) = {r2_cur - r2_market_eval:+.4f}"
    )
    print(
        f"  参考: CORP MCB 候補 {corp_cand['mcb']:.5f} vs 現行 {corp_cur['mcb']:.5f}"
        f"（副 KPI・採否ゲート外）"
    )
    print(f"\n測定判定: {'採用（測定上）' if verdict else '棄却（現行維持）'}")
    if verdict and cand_alpha == 0.0:
        print("※ α=0 は製品判断（◎・混戦判定が市場確率ベース化）を伴うため、最終採否は PO 承認が必要。")


if __name__ == "__main__":
    main()
