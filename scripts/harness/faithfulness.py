#!/usr/bin/env python3
"""学習型モデル評価ハーネス③（#272 / #309）の忠実性サニティ。

`analyze backtest --dump-features` が出力する TSV を集計し、内蔵モデルの予測列
（model_win/place/show）から的中率・回収率・Brier・LogLoss を再計算する。これを
`analyze backtest` 本体の出力と突合し、**Python 評価ハーネスが Rust backtest と同じ数値を
出すこと**を確認する（ハーネスのバグ・設定差を検出する回帰ゲート）。これが通って初めて、
学習モデル（#309 Phase B）の評価をこのハーネスで信頼できる。

依存は標準ライブラリのみ（このリポの Python 規約に合わせる）。集計ロジックは Rust 側
`src/domain/src/backtest/{evaluate,metrics}.rs` を忠実に移したもの:
  - トップ選好馬 = model_win 最大（同値は馬番昇順）。
  - 的中率（単勝/連対/複勝）= トップ選好馬の確定着順が 1 / ≤2 / ≤3。母数は評価レース数。
  - 想定回収率 = Σ(的中時オッズ) / オッズ取得レース数（賭金一定 100 円は約分）。
  - Brier/LogLoss = 全出走馬エントリ平均。LogLoss は ε=1e-15 クランプ。
"""

import argparse
import csv
import math
import re
import sys
from collections import defaultdict

# domain backtest/metrics.rs の LOG_LOSS_EPS と一致させる（ln(0) 発散防止のクランプ）。
LOG_LOSS_EPS = 1e-15


def load_dump(path):
    """ダンプ TSV を行 dict のリストに読む（ヘッダ駆動なので列順変更に頑健）。"""
    with open(path, newline="", encoding="utf-8") as f:
        return list(csv.DictReader(f, delimiter="\t"))


def _opt_int(s):
    return int(s) if s not in ("", None) else None


def _opt_float(s):
    return float(s) if s not in ("", None) else None


def compute_metrics(rows):
    """ダンプ行から backtest と同じ集計指標を再計算して dict で返す。"""
    by_race = defaultdict(list)
    for r in rows:
        by_race[r["race_id"]].append(r)
    n_races = len(by_race)
    if n_races == 0:
        raise ValueError("ダンプにレースが 1 件も無い")

    win_hits = place_hits = show_hits = 0
    total_payout = 0.0
    payout_races = 0
    for horses in by_race.values():
        # トップ選好馬: model_win 最大、同値は馬番昇順（Rust の reduce と同規則）。
        top = min(horses, key=lambda h: (-float(h["model_win"]), int(h["horse_num"])))
        fp = _opt_int(top["finishing_position"])
        if fp is not None:
            if fp == 1:
                win_hits += 1
            if fp <= 2:
                place_hits += 1
            if fp <= 3:
                show_hits += 1
        odds = _opt_float(top["win_odds"])
        if odds is not None:
            payout_races += 1
            if fp == 1:
                total_payout += odds

    def calibration(prob_key, hit_pred):
        brier_sum = 0.0
        log_loss_sum = 0.0
        for r in rows:
            p = float(r[prob_key])
            fp = _opt_int(r["finishing_position"])
            y = 1.0 if hit_pred(fp) else 0.0
            brier_sum += (p - y) ** 2
            pc = min(max(p, LOG_LOSS_EPS), 1.0 - LOG_LOSS_EPS)
            log_loss_sum += -(y * math.log(pc) + (1.0 - y) * math.log(1.0 - pc))
        n = len(rows)
        return (brier_sum / n, log_loss_sum / n) if n else (0.0, 0.0)

    brier_win, ll_win = calibration("model_win", lambda fp: fp == 1)
    brier_place, ll_place = calibration("model_place", lambda fp: fp is not None and fp <= 2)
    brier_show, ll_show = calibration("model_show", lambda fp: fp is not None and fp <= 3)

    return {
        "races": n_races,
        "win_hit": win_hits / n_races,
        "place_hit": place_hits / n_races,
        "show_hit": show_hits / n_races,
        "payout_rate": (total_payout / payout_races) if payout_races else None,
        "payout_races": payout_races,
        "brier_win": brier_win,
        "brier_place": brier_place,
        "brier_show": brier_show,
        "logloss_win": ll_win,
        "logloss_place": ll_place,
        "logloss_show": ll_show,
    }


# 常に印字される＝抽出できなければレポートのフォーマットがドリフトしたとみなしてゲートを
# hard fail させる必須キー（パース退行で「偽 OK」になる穴を塞ぐ）。`payout_rate` はオッズ取得
# レースが 0 件だと backtest が「—」を印字し正当に欠落しうるので、ここには含めず main で
# payout_races>0 のときだけ要求する。
REQUIRED_REPORT_KEYS = (
    "races",
    "win_hit",
    "place_hit",
    "show_hit",
    "payout_races",
    "brier_win",
    "brier_place",
    "brier_show",
    "logloss_win",
    "logloss_place",
    "logloss_show",
)


def parse_backtest_report(text):
    """`analyze backtest` の標準出力から突合用の数値を抽出する。

    印字は的中率/回収率が小数1桁の%、Brier/LogLoss が小数4桁。丸め後の値を返すため、
    突合は印字桁に合わせた許容で行う（[`compare`] 参照）。抽出できなかったキーは `None`。
    呼び出し側は必須キーの欠落を hard fail として扱う（[`REQUIRED_REPORT_KEYS`] / [`main`]）。
    """
    out = {}

    def pct(label):
        m = re.search(rf"{label}\s*[:：]\s*([\d.]+)%", text)
        return float(m.group(1)) / 100.0 if m else None

    m_races = re.search(r"評価レース数\s*[:：]\s*(\d+)", text)
    out["races"] = int(m_races.group(1)) if m_races else None
    out["win_hit"] = pct("単勝的中率")
    out["place_hit"] = pct("連対的中率")
    out["show_hit"] = pct("複勝的中率")
    # 母数（オッズ取得レース数）は回収率が「—」でも常に印字されるので独立に取る。回収率本体は
    # 0 件時に「—」になり数値が無いため、`%` 形のみ拾う（無ければ None で、母数 0 のときは正当）。
    m_races_pay = re.search(r"\(母数\s*(\d+)\s*レース\)", text)
    out["payout_races"] = int(m_races_pay.group(1)) if m_races_pay else None
    m_rate = re.search(r"想定回収率\s*[:：]\s*([\d.]+)%", text)
    out["payout_rate"] = float(m_rate.group(1)) / 100.0 if m_rate else None

    # Brier/LogLoss テーブル（「単勝 0.0589 0.2104」等）は「## 確率校正」セクション以降に限定して
    # 行頭アンカーで拾う。reliability 等の後続表や的中率行を誤って拾わないようにするため。
    cal_idx = text.find("確率校正")
    cal_section = text[cal_idx:] if cal_idx != -1 else text
    for jp, key in (("単勝", "win"), ("連対", "place"), ("複勝", "show")):
        m = re.search(rf"^{jp}\s+([\d.]+)\s+([\d.]+)", cal_section, re.MULTILINE)
        out[f"brier_{key}"] = float(m.group(1)) if m else None
        out[f"logloss_{key}"] = float(m.group(2)) if m else None
    return out


def compare(computed, expected):
    """印字桁に合わせた許容で突合し、不一致 (キー, 計算値, 期待値, 許容) のリストを返す。"""
    # 的中率・回収率は小数1桁%印字 → 0.05% の半桁許容。Brier/LogLoss は小数4桁 → 5e-5。
    tol = {
        "win_hit": 5e-4,
        "place_hit": 5e-4,
        "show_hit": 5e-4,
        "payout_rate": 5e-4,
        "brier_win": 5e-5,
        "brier_place": 5e-5,
        "brier_show": 5e-5,
        "logloss_win": 5e-5,
        "logloss_place": 5e-5,
        "logloss_show": 5e-5,
    }
    mismatches = []
    # 整数の母数（評価レース数・オッズ取得レース数）は許容ゼロで厳密一致を要求する。
    for key in ("races", "payout_races"):
        exp = expected.get(key)
        if exp is not None and exp != computed.get(key):
            mismatches.append((key, computed.get(key), exp, 0))
    for key, t in tol.items():
        exp = expected.get(key)
        got = computed.get(key)
        if exp is None or got is None:
            continue
        if abs(got - exp) > t:
            mismatches.append((key, got, exp, t))
    return mismatches


def _print_metrics(m):
    print(f"評価レース数        : {m['races']}")
    print(f"単勝的中率          : {m['win_hit'] * 100:.1f}%")
    print(f"連対的中率          : {m['place_hit'] * 100:.1f}%")
    print(f"複勝的中率          : {m['show_hit'] * 100:.1f}%")
    pr = m["payout_rate"]
    print(
        f"想定回収率          : {pr * 100:.1f}%  (母数 {m['payout_races']} レース)"
        if pr is not None
        else "想定回収率          : なし"
    )
    print("種別     Brier    LogLoss")
    for jp, key in (("単勝", "win"), ("連対", "place"), ("複勝", "show")):
        print(f"{jp}     {m[f'brier_{key}']:.4f}     {m[f'logloss_{key}']:.4f}")


def main(argv=None):
    ap = argparse.ArgumentParser(description="特徴量ダンプの忠実性サニティ（#272/#309）")
    ap.add_argument("dump", help="analyze backtest --dump-features の出力 TSV")
    ap.add_argument(
        "--backtest-report",
        help="analyze backtest の標準出力を保存したファイル。指定すると突合し、"
        "不一致があれば終了コード 1。",
    )
    args = ap.parse_args(argv)

    rows = load_dump(args.dump)
    metrics = compute_metrics(rows)
    _print_metrics(metrics)

    if not args.backtest_report:
        return 0

    with open(args.backtest_report, encoding="utf-8") as f:
        expected = parse_backtest_report(f.read())
    # パース退行（backtest 出力フォーマットの変化）を「偽 OK」にしないため、必須キーが 1 つでも
    # 取れなければ突合せず hard fail する。これがゲートの堅牢性の要（#312 レビュー C1）。
    missing = [k for k in REQUIRED_REPORT_KEYS if expected.get(k) is None]
    # オッズ取得レースがあるのに回収率が拾えないのはパース退行（0 件時は「—」で正当に欠落）。
    if expected.get("payout_races") and expected.get("payout_rate") is None:
        missing.append("payout_rate")
    if missing:
        print(
            f"\n忠実性サニティ NG: backtest レポートのパースに失敗（未抽出キー: {', '.join(missing)}）。"
            "出力フォーマットが変わった可能性。",
            file=sys.stderr,
        )
        return 1
    mismatches = compare(metrics, expected)
    if mismatches:
        print("\n忠実性サニティ NG（ハーネスが backtest と不一致）:", file=sys.stderr)
        for key, got, exp, t in mismatches:
            print(f"  {key}: 計算 {got} vs backtest {exp} (許容 {t})", file=sys.stderr)
        return 1
    print("\n忠実性サニティ OK: ハーネスの集計が analyze backtest と一致")
    return 0


if __name__ == "__main__":
    sys.exit(main())
