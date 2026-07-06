#!/usr/bin/env python3
"""strategy_eval の買い目組み立て（券種別相手頭数・後方互換・100 円単位配分）の回帰テスト。

#347 で追加した `--wide-partners`（ワイドだけ別頭数）が、馬連・三連複を top5 に据えたまま
ワイドの点数だけ独立して変わること、および未指定時（wide_partners==partners）は従来出力に
帰着することを固定する。計測ハーネスは ADR 0065 の意思決定根拠なのでリグレッションを防ぐ。
"""
import json
import os
import subprocess
import sys
import tempfile

import strategy_eval as S

_HERE = os.path.dirname(os.path.abspath(__file__))


def _labels(bets):
    return {want: [c for lbl, c, _ in bets if lbl == want] for want in ("quinella", "wide", "trio")}


def test_wide_partners_narrower_than_others():
    # ワイド top3 / 馬連・三連複 top5。ワイドだけ 3 点、馬連 5 点、三連複 C(5,2)=10 点。
    axis = 1
    partners = [2, 3, 4, 5, 6]
    wide_partners = [2, 3, 4]
    bets = S.build_bets(axis, partners, wide_partners, 5000, (1, 1, 1), with_trio=True)
    g = _labels(bets)
    assert len(g["wide"]) == 3, g["wide"]
    assert len(g["quinella"]) == 5, g["quinella"]
    assert len(g["trio"]) == 10, g["trio"]
    # ワイドの組は wide_partners 由来（相手 5,6 を含まない）
    assert g["wide"] == [S.code_unordered([axis, p]) for p in wide_partners]


def test_wide_partners_backward_compat():
    # wide_partners == partners（--wide-partners 未指定に相当）なら馬連とワイドは同一組・同点数。
    axis = 1
    partners = [2, 3, 4, 5, 6]
    bets = S.build_bets(axis, partners, partners, 5000, (1, 1, 1), with_trio=True)
    g = _labels(bets)
    assert g["wide"] == g["quinella"]
    assert len(g["wide"]) == 5


def test_wide_partners_wider_than_others():
    # ワイド top5 / 馬連 top3 も破綻せず組める（wide が other より広い設定も許容）。
    axis = 1
    bets = S.build_bets(axis, [2, 3, 4], [2, 3, 4, 5, 6], 5000, (1, 1, 1), with_trio=True)
    g = _labels(bets)
    assert len(g["wide"]) == 5
    assert len(g["quinella"]) == 3
    assert len(g["trio"]) == 3  # C(3,2)


def test_distribute_100yen_units():
    # 100 円単位均等割り。賄えない端数は買わない（build_portfolio.distribute と同流儀）。
    assert S.distribute(1500, 5) == [300] * 5
    assert S.distribute(250, 3) == [100, 100, 0]  # per<100 → 100 円で買える点数だけ
    assert S.distribute(1600, 3) == [500, 500, 500]  # 1600//3//100*100=500
    assert S.distribute(50, 2) == [0, 0]  # budget<100


def test_no_trio_when_disabled():
    # with_trio=False は三連複を出さない（馬連＋ワイドのみ）。
    bets = S.build_bets(1, [2, 3, 4, 5, 6], [2, 3, 4, 5, 6], 5000, (1, 1, 1), with_trio=False)
    assert all(lbl != "trio" for lbl, _, _ in bets)


def _run_json(tmp, extra):
    """--json で strategy_eval を CLI 実行し、(returncode, parsed_json_or_None) を返す。"""
    preds = os.path.join(tmp, "preds.json")
    payouts = os.path.join(tmp, "payouts.json")
    with open(preds, "w", encoding="utf-8") as f:
        json.dump([{"venue": "東京", "race_num": 11, "horses": [
            {"num": n, "win": w} for n, w in
            [(1, 0.30), (2, 0.20), (3, 0.15), (4, 0.12), (5, 0.10), (6, 0.08), (7, 0.05)]]}], f)
    with open(payouts, "w", encoding="utf-8") as f:
        json.dump([{"venue_jp": "東京", "race_num": 11,
                    "payouts": {"wide": {"1-6": 1200}, "quinella": {}, "trio": {}, "win": {}}}], f)
    p = subprocess.run(
        [sys.executable, os.path.join(_HERE, "strategy_eval.py"), preds, payouts,
         "--budget", "5000", *extra],
        cwd=_HERE, capture_output=True, text=True)
    try:
        return p.returncode, json.loads(p.stdout)
    except json.JSONDecodeError:
        return p.returncode, None


def test_json_output_shape():
    # --json 集計パス（多日集計 = ADR 0065 の意思決定根拠機構）の出力形状を固定する。
    with tempfile.TemporaryDirectory() as tmp:
        rc, out = _run_json(tmp, ["--partners", "5", "--wide-partners", "3", "--json"])
        assert rc == 0 and out is not None
        assert out["budget"] == 5000 and out["partners"] == 5 and out["wide_partners"] == 3
        assert out["races"] == 1
        strat = out["strategies"]
        assert set(strat) == {"本命単勝のみ", "本命軸 馬連+ワイド流し", "本命軸 馬連+ワイド+三連複流し"}
        for cell in strat.values():
            assert isinstance(cell["stake"], int) and isinstance(cell["payout"], int)
        # 未指定なら wide_partners は partners に追従する
        rc2, out2 = _run_json(tmp, ["--partners", "5", "--json"])
        assert rc2 == 0 and out2["wide_partners"] == 5


def test_json_rejects_multi_k():
    # --json は単一 K のみ（多日集計で合算するため）。複数Kは exit 1。
    with tempfile.TemporaryDirectory() as tmp:
        rc, out = _run_json(tmp, ["--partners", "3,5", "--json"])
        assert rc == 1


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")


if __name__ == "__main__":
    main()
