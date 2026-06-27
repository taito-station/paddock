"""umaren_backtest.py の馬単(exacta)拡張の最小テスト（pytest 不要・`python3 test_umaren_backtest.py`）.

#262 で追加した順序付き Plackett-Luce（p_exacta）と入力パース（exacta は '>' 順序キー）の
不変量・契約を assert で固定する。デグレに気付きにくい数値ロジックの回帰検出が目的。
"""
import os
import tempfile
from itertools import permutations

import umaren_backtest as U


def approx(a, b, eps=1e-9):
    return abs(a - b) < eps


def test_p_exacta_invariants():
    # 不均等 4 頭フィールド
    probs = {1: 40.0, 2: 30.0, 3: 20.0, 4: 10.0}
    horses = list(probs)
    # 全順序ペア（1着>2着）の総和は 1（ちょうど 1 つの順序ペアが上位 2 着を占める）
    s = sum(U.p_exacta(probs, a, b) for a, b in permutations(horses, 2))
    assert approx(s, 1.0), s
    # 馬連との整合: 順序 2 レッグの和 == 無順 馬連確率
    for a, b in permutations(horses, 2):
        assert approx(U.p_exacta(probs, a, b) + U.p_exacta(probs, b, a), U.p_top2_set(probs, a, b))
    # 非対称性: 強い馬が 1 着の順序の方が確率が高い
    assert U.p_exacta(probs, 1, 4) > U.p_exacta(probs, 4, 1)
    # [0,1] かつ縮退ガード（z-pa<=0 で 0）
    assert all(0.0 <= U.p_exacta(probs, a, b) <= 1.0 for a, b in permutations(horses, 2))
    assert U.p_exacta({1: 100.0}, 1, 1) == 0.0  # z-pa==0 ガード


def test_parse_exotic_exacta_ordered():
    # exacta は '>' 順序キー→(1着,2着)タプル、無順券種は '-'→frozenset
    sample = (
        "pid1\tquinella\t3-5\t12.4\n"
        "pid1\ttrio\t3-5-7\t88.0\n"
        "pid1\texacta\t3>5\t25.1\n"
        "pid1\texacta\t5>3\t40.7\n"
        "pid1\tunknown\t9-9\t1.0\n"  # 想定外 bet_type は無視される
    )
    fd, path = tempfile.mkstemp(suffix=".tsv")
    try:
        with os.fdopen(fd, "w") as f:
            f.write(sample)
        out = U.parse_exotic(path)
    finally:
        os.unlink(path)
    slot = out["pid1"]
    assert slot["quinella"][frozenset({3, 5})] == 12.4
    assert slot["trio"][frozenset({3, 5, 7})] == 88.0
    # 順序が保持され、向きで別エントリになる
    assert slot["exacta"][(3, 5)] == 25.1
    assert slot["exacta"][(5, 3)] == 40.7
    assert (5, 3) != (3, 5)


def test_eval_exacta_only_uses_ordered_payout():
    # ◎=1。(1>2) のみ +EV になるオッズ盤面で、的中は実払戻(1,2)から取れること。
    probs = {1: 50.0, 2: 30.0, 3: 20.0}
    exacta_odds = {(1, 2): 100.0, (2, 1): 1.0, (1, 3): 1.0, (3, 1): 1.0}
    pay = {"umaren": {}, "wide": {}, "trio": {}, "exacta": {(1, 2): 10000}}
    bet, ret, stake = U.eval_exacta_only(probs, exacta_odds, pay, theta=1.0, mode="flat")
    assert bet and stake == 5000
    # (1>2) が当たり、(1,2) の払戻でリターンが発生する
    assert ret == 5000 // 100 * 10000
    # exacta 盤面が空なら個別スキップ（bet=False）
    bet2, _, _ = U.eval_exacta_only(probs, {}, pay, theta=1.0, mode="flat")
    assert not bet2


def test_parse_result_exacta_ordered_key():
    # 馬単(Umatan)行は (1着,2着) タプルでキー化、無順券種(Umaren)は frozenset
    html = (
        '<tr class="Umaren"><td class="Result"><ul><li>6</li><li>4</li></ul></td>'
        '<td class="Payout">3,830円</td></tr>'
        '<tr class="Umatan"><td class="Result"><ul><li>6</li><li>4</li></ul></td>'
        '<td class="Payout">7,210円</td></tr>'
    )
    fd, path = tempfile.mkstemp(suffix=".html")
    try:
        with os.fdopen(fd, "w") as f:
            f.write(html)
        _, pay = U.parse_result(path)
    finally:
        os.unlink(path)
    assert pay["umaren"][frozenset({6, 4})] == 3830
    assert pay["exacta"][(6, 4)] == 7210  # 出現順=1着>2着
    assert (4, 6) not in pay["exacta"]  # 逆順は別物


def test_eval_pair_leg_swap_replaces_only_ev_favorable_legs():
    # ADR 0043 pick_pair_leg 直接検証の中核。◎=1。
    # pair(1,2): 馬単オッズ高=EV優位→置換 / pair(1,3): 馬単オッズ低=非優位→馬連維持 /
    # pair(1,4): 馬単オッズ欠落→馬連維持。置換は1脚のみのはず。
    probs = {1: 40.0, 2: 30.0, 3: 20.0, 4: 10.0}
    quin_odds = {frozenset({1, 2}): 1.0, frozenset({1, 3}): 1.0, frozenset({1, 4}): 1.0}
    exacta_odds = {(1, 2): 100.0, (1, 3): 1.0}  # (1,4) は欠落
    pay = {
        "umaren": {frozenset({1, 2}): 500},  # 馬連は (1,2) のみ的中
        "wide": {}, "trio": {},
        "exacta": {(1, 2): 1000},  # 馬単も (1,2) 的中
    }
    # swap=False: 全ペア馬連で清算（土台）。配分 lr([30,20,10],15)=[7,5,3]→¥700/500/300。
    bet, ret, stake, sw = U.eval_pair_leg_swap(probs, quin_odds, exacta_odds, pay, swap=False)
    assert bet and stake == 1500 and sw == 0
    assert ret == 700 * 500 // 100  # 3500（馬連 (1,2) のみ）
    # swap=True: (1,2) のみ馬単へ置換。(1,3)=非優位/(1,4)=欠落は馬連維持。
    bet, ret, stake, sw = U.eval_pair_leg_swap(probs, quin_odds, exacta_odds, pay, swap=True)
    assert bet and stake == 1500 and sw == 1, sw
    assert ret == 700 * 1000 // 100  # 7000（馬単 (1,2) 払戻で清算）


def test_eval_exacta_plain_weighted_both_directions():
    # top5 両方向・馬単確率重み（ADR の「馬単top5 67.6%」を生む load-bearing 経路）。
    # 両方向 universe・p_exacta 重み・lr 配分・exacta 清算の契約を固定する。
    probs = {1: 50.0, 2: 30.0, 3: 20.0}
    pay = {"umaren": {}, "wide": {}, "trio": {}, "exacta": {(1, 2): 1000}}
    A, partners = 1, [2, 3]
    combos = [c for p in partners for c in ((A, p), (p, A))]  # (1,2)(2,1)(1,3)(3,1)
    weights = [U.p_exacta(probs, c[0], c[1]) for c in combos]
    units = U.largest_remainder(weights, 1500 // 100)
    exp_ret = sum(u * 100 * pay["exacta"].get(c, 0) // 100 for c, u in zip(combos, units))
    assert U.eval_exacta_plain(probs, pay, budget=1500) == (True, exp_ret, 1500)
    # 重みは p_exacta（無順 p_top2_set ではない）: 強い向き (1,2) が逆向き (2,1) より厚い
    assert units[0] > units[1]


def test_eval_exacta_allflat_buys_both_directions():
    # 全頭両方向 flat。◎=1, 相手 [2,3] → 4 レッグ (1,2)(2,1)(1,3)(3,1)。予算ちょうど・実払戻清算。
    probs = {1: 50.0, 2: 30.0, 3: 20.0}
    pay = {"umaren": {}, "wide": {}, "trio": {}, "exacta": {(1, 2): 1000}}
    bet, ret, stake = U.eval_exacta_allflat(probs, pay, budget=5000)
    assert bet and stake == 5000
    # lr([1,1,1,1],50)=[13,13,12,12]。(1,2) は最初のレッグ=¥1300。
    assert ret == 1300 * 1000 // 100  # 13000


def test_plain_truncates_to_top5_but_allflat_keeps_all():
    # 7 頭立て: plain は相手 top5（ranked[1:6]）に切り詰め、allflat は全頭（ranked[1:]）。
    # 第7頭（最下位）絡みの脚だけ的中させ、plain は買わず allflat は買うことで top5 切り詰めを固定。
    probs = {1: 30.0, 2: 20.0, 3: 15.0, 4: 12.0, 5: 10.0, 6: 8.0, 7: 5.0}
    pay = {"umaren": {}, "wide": {}, "trio": {}, "exacta": {(1, 7): 1000}}  # ◎→第7頭のみ的中
    bet_p, ret_p, stake_p = U.eval_exacta_plain(probs, pay, budget=5000)
    bet_a, ret_a, stake_a = U.eval_exacta_allflat(probs, pay, budget=5000)
    assert bet_p and stake_p == 5000 and ret_p == 0  # top5 は第7頭の脚を買わない→不的中
    assert bet_a and stake_a == 5000 and ret_a > 0   # 全頭は (1,7) を買う→的中


# --- #270: 確率→EV パイプライン再計算（α×γ）の不変量 -------------------------
def test_market_implied_normalizes_and_excludes_no_odds():
    # オッズ有る馬の implied は overround 正規化で Σ1。オッズ<=0 / 欠落馬は除外。
    winodds_pid = {1: (1, 2.0), 2: (2, 4.0), 3: (3, 4.0), 4: (4, 0.0)}
    imp = U.market_implied(winodds_pid)
    assert 4 not in imp  # odds=0 は市場確率を持たない
    assert approx(sum(imp.values()), 1.0)
    # raw=0.5,0.25,0.25 → overround=1.0 → そのまま implied
    assert approx(imp[1], 0.5) and approx(imp[2], 0.25) and approx(imp[3], 0.25)
    # overround>1（控除）の例: 全馬 odds=1.0 → raw=1 ×3 → 各 1/3
    imp2 = U.market_implied({1: (1, 1.0), 2: (2, 1.0), 3: (3, 1.0)})
    assert all(approx(v, 1 / 3) for v in imp2.values())
    assert U.market_implied({}) == {}


def test_recompute_p_final_matches_rust_order_fixture():
    # 手計算した Rust 順序（blend→正規化→冪→正規化）の固定値と一致すること。
    # p_model = {1:50,2:30,3:20}%、implied = {1:0.2,2:0.3,3:0.5}、α=0.5、γ=1.0。
    # blended = 0.5*model + 0.5*implied = {1:0.35,2:0.30,3:0.35}（Σ=1.0）。
    # γ=1.0 なので冪は恒等 → final% = {35,30,35}。
    pmodel = {1: 50.0, 2: 30.0, 3: 20.0}
    implied = {1: 0.2, 2: 0.3, 3: 0.5}
    out = U.recompute_p_final(pmodel, implied, alpha=0.5, gamma=1.0)
    assert approx(sum(out.values()), 100.0)
    assert approx(out[1], 35.0) and approx(out[2], 30.0) and approx(out[3], 35.0)


def test_recompute_alpha1_has_no_market_term():
    # α=1.0 では (1-α)*implied 項が消え、output = normalize(model**γ)。implied は無視される。
    pmodel = {1: 50.0, 2: 30.0, 3: 20.0}
    implied = {1: 0.99, 2: 0.005, 3: 0.005}  # 極端な市場でも α=1.0 なら効かない
    a = U.recompute_p_final(pmodel, implied, alpha=1.0, gamma=1.0)
    assert approx(a[1], 50.0) and approx(a[2], 30.0) and approx(a[3], 20.0)
    # γ=1.0・α=1.0 は恒等（model をそのまま返す）
    b = U.recompute_p_final(pmodel, {}, alpha=1.0, gamma=1.0)
    assert all(approx(a[k], b[k]) for k in pmodel)


def test_recompute_gamma_normalized_and_in_unit():
    # 任意 (α,γ) で出力は Σ1（%なら Σ100）かつ各値 [0,100]。
    pmodel = {1: 50.0, 2: 30.0, 3: 15.0, 4: 5.0}
    implied = {1: 0.4, 2: 0.3, 3: 0.2, 4: 0.1}
    for alpha in (0.0, 0.2, 0.5, 1.0):
        for gamma in (1.0, 1.25, 2.0):
            out = U.recompute_p_final(pmodel, implied, alpha, gamma)
            assert approx(sum(out.values()), 100.0), (alpha, gamma)
            assert all(0.0 <= v <= 100.0 for v in out.values())


def test_recompute_gamma_up_concentrates_top_mass():
    # γ を上げると上位馬の確率質量が増える（冪較正の方向性）。
    pmodel = {1: 50.0, 2: 30.0, 3: 20.0}
    lo = U.recompute_p_final(pmodel, {}, alpha=1.0, gamma=1.0)
    hi = U.recompute_p_final(pmodel, {}, alpha=1.0, gamma=2.0)
    assert hi[1] > lo[1]  # トップは厚く
    assert hi[3] < lo[3]  # ボトムは薄く


def test_recover_p_model_roundtrip():
    # α=1.0 の最終確率 = normalize(p_model**γ)。recover で元の p_model に戻ること。
    pmodel = {1: 45.0, 2: 25.0, 3: 18.0, 4: 12.0}
    final = U.recompute_p_final(pmodel, {}, alpha=1.0, gamma=1.25)
    rec = U.recover_p_model(final, gamma=1.25)
    assert approx(sum(rec.values()), 100.0)
    assert all(approx(rec[k], pmodel[k], eps=1e-6) for k in pmodel), rec


def test_top1_topk_brier_on_known_winner():
    probs = {1: 50.0, 2: 30.0, 3: 20.0}
    assert U.top1_hit(probs, 1) == 1
    assert U.top1_hit(probs, 2) == 0
    assert U.topk_recall(probs, 3, 1) == 0 and U.topk_recall(probs, 3, 3) == 1
    assert U.topk_recall(probs, 2, 2) == 1
    # Brier = mean_h (p-y)^2 = ((.5-1)^2+(.3)^2+(.2)^2)/3 = (0.25+0.09+0.04)/3
    assert approx(U.brier(probs, 1), (0.25 + 0.09 + 0.04) / 3)


def test_spearman_monotonic_and_degenerate():
    assert approx(U.spearman([1, 2, 3, 4], [10, 20, 30, 40]), 1.0)
    assert approx(U.spearman([1, 2, 3, 4], [40, 30, 20, 10]), -1.0)
    assert U.spearman([1], [1]) == 0.0  # n<2
    assert U.spearman([5, 5, 5], [1, 2, 3]) == 0.0  # 分散ゼロ
    # 同順位（平均順位）: 2,2 のタイがあっても破綻しない（-1<=r<=1）
    r = U.spearman([1, 2, 2, 3], [1, 2, 3, 4])
    assert -1.0 <= r <= 1.0


def test_race_winner_from_exacta():
    pay = {"exacta": {(7, 3): 1000, (7, 5): 2000}}  # 1着=7 が共通
    assert U.race_winner(pay) == 7
    assert U.race_winner({"exacta": {}}) is None
    assert U.race_winner({"exacta": {(1, 2): 100, (3, 4): 200}}) is None  # 同着1着→曖昧


def test_calibration_buckets_counts_and_realized():
    # 合成 evaluated 2 鞍で bucket 件数・実現 ROI が想定どおりか（stdout を捕捉）。
    # 馬連のみ盤面（trio 空）で ◎=1 軸ながし。1 鞍は (1,2) 的中、もう 1 鞍は不的中。
    import io
    import contextlib
    probs = {1: 50.0, 2: 30.0, 3: 20.0}
    quin = {frozenset({1, 2}): 5.0, frozenset({1, 3}): 5.0}
    trio = {}
    pay_hit = {"umaren": {frozenset({1, 2}): 10000}, "wide": {}, "trio": {}, "exacta": {(1, 2): 1}}
    pay_miss = {"umaren": {}, "wide": {}, "trio": {}, "exacta": {(9, 8): 1}}
    evaluated = [
        ("d1", "東京", 1, "p1", probs, quin, trio, {}, pay_hit),
        ("d1", "東京", 2, "p2", probs, quin, trio, {}, pay_miss),
    ]
    probs_by_race = {("d1", "東京", 1): probs, ("d1", "東京", 2): probs}
    # 両鞍とも同 probs/odds なので model_roi が一致 → 同一 bucket に 2 件入る。
    mr, ret_hit, stake = U.compute_baseline_pf(probs, quin, trio, pay_hit)
    _, ret_miss, _ = U.compute_baseline_pf(probs, quin, trio, pay_miss)
    edges = [mr - 0.01]  # 2 件とも上側 bucket に入る境界
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        U.calibration_buckets(evaluated, probs_by_race, edges)
    out = buf.getvalue()
    # 上側 bucket に n=2、下側 bucket は n=0
    assert f">={(mr - 0.01) * 100:.0f}%" in out
    # 実現 ROI = (ret_hit + ret_miss) / (2*stake)
    realized = (ret_hit + ret_miss) / (2 * stake) * 100
    assert f"{realized:.1f}%" in out, (realized, out)
    # 的中 1/2 = 50%
    assert "50%" in out


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")


if __name__ == "__main__":
    main()
