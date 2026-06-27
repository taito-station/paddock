"""snapshot_ev_report.py の純関数テスト（pytest 不要・`python3 test_snapshot_ev.py` で実行）.

DB / analyze に触れない group_snapshots / eval_race / load_snapshot_rows の不変量を固定する。
ROI ロジック自体は live_ev 側（test_live_ev.py）で担保済みなので、ここでは
「snapshot 構造 → 時系列走査 → ever/final 判定」と「ワイド mid 変換」の正しさに絞る。
"""
import snapshot_ev_report as S


def approx(a, b, eps=1e-9):
    return abs(a - b) < eps


def _row(rid, bt, key, odds, odds_high="", at="2026-06-27T00:00:00+00:00",
         date="2026-06-27", venue="hakodate", rnum="1", surface="turf", dist="1200"):
    return dict(race_id=rid, date=date, venue=venue, race_num=rnum, surface=surface,
                distance=dist, bet_type=bt, combination_key=key, odds=str(odds),
                odds_high=str(odds_high), fetched_at=at)


def test_group_snapshots_wide_mid_and_keys():
    # ワイドは low(odds)/high(odds_high) から mid を採る。馬連/3連複は単一オッズ。
    rows = [
        _row("R1", "win", "1", 3.5),
        _row("R1", "quinella", "1-2", 12.4),
        _row("R1", "trio", "3-1-2", 88.0),   # 非ソート入力でも昇順 tuple に正規化
        _row("R1", "wide", "2-1", 3.0, odds_high=5.0),
    ]
    races = S.group_snapshots(rows)
    assert set(races) == {"R1"}, races
    books = races["R1"]["times"]["2026-06-27T00:00:00+00:00"]
    assert books["win"] == {1: 3.5}
    assert books["quinella"] == {(1, 2): 12.4}
    assert books["trio"] == {(1, 2, 3): 88.0}
    assert approx(books["wide"][(1, 2)], 4.0), books["wide"]  # (3+5)/2
    assert races["R1"]["race_num"] == 1


def test_group_snapshots_wide_missing_high_skipped():
    # odds_high 欠落のワイドは mid を出せないのでスキップ（KeyError にしない）。
    races = S.group_snapshots([_row("R1", "wide", "1-2", 3.0, odds_high="")])
    books = races["R1"]["times"]["2026-06-27T00:00:00+00:00"]
    assert books["wide"] == {}, books["wide"]


def test_group_snapshots_multiple_times():
    # 同一レースの別 fetched_at は別 snapshot 時点として分かれる。
    rows = [
        _row("R1", "win", "1", 3.5, at="2026-06-27T00:00:00+00:00"),
        _row("R1", "win", "1", 3.2, at="2026-06-27T01:00:00+00:00"),
    ]
    races = S.group_snapshots(rows)
    assert set(races["R1"]["times"]) == {
        "2026-06-27T00:00:00+00:00", "2026-06-27T01:00:00+00:00"}


def test_load_snapshot_rows_column_guard():
    good = "\t".join(["R1", "2026-06-27", "hakodate", "1", "turf", "1200",
                      "win", "1", "3.5", "", "2026-06-27T00:00:00+00:00"])
    bad = "R1\twin\t3.5"  # 列数不足は捨てる
    rows = S.load_snapshot_rows(good + "\n" + bad + "\n")
    assert len(rows) == 1 and rows[0]["bet_type"] == "win", rows


class ConstBook(dict):
    """任意の組番に対し固定オッズを返すテスト用 book（bets の組番を事前に知らずに済む）。"""

    def __init__(self, value):
        super().__init__()
        self._value = value

    def get(self, _key, _default=None):
        return self._value


def _times_with(const_by_time):
    """{fetched_at: const_odds} から eval_race 用の times（wide/quinella/trio を ConstBook）を作る。"""
    times = {}
    for at, val in const_by_time.items():
        times[at] = {"win": {1: 2.0, 2: 3.0, 3: 4.0, 4: 5.0},
                     "quinella": ConstBook(val), "trio": ConstBook(val), "wide": ConstBook(val)}
    return times


def test_eval_race_ever_but_not_final():
    # 早い時点は高オッズ(+EV)、最終時点は配当0(−EV) → ever_pos=True, final_pos=False。
    probs = {1: 40.0, 2: 30.0, 3: 20.0, 4: 10.0}
    times = _times_with({
        "2026-06-27T00:00:00+00:00": 1000.0,  # 高配当 → ROI 大
        "2026-06-27T01:00:00+00:00": 0.0,      # 配当なし → ROI 0
    })
    ev = S.eval_race(probs, times, budget=5000)
    assert ev["ever_pos"] is True, ev
    assert ev["final_pos"] is False, ev
    assert ev["final_at"] == "2026-06-27T01:00:00+00:00", ev  # 最終=最遅 fetched_at
    assert ev["n_times"] == 2


def test_eval_race_final_pos():
    # 最終時点が高配当なら final_pos=True。
    probs = {1: 40.0, 2: 30.0, 3: 20.0, 4: 10.0}
    times = _times_with({
        "2026-06-27T00:00:00+00:00": 0.0,
        "2026-06-27T01:00:00+00:00": 1000.0,
    })
    ev = S.eval_race(probs, times, budget=5000)
    assert ev["final_pos"] is True and ev["ever_pos"] is True, ev


def test_eval_race_degenerate_returns_none():
    # 出走馬 < 3 は買い目が組めず None（集計対象外）。
    assert S.eval_race({1: 50.0, 2: 50.0}, _times_with({"t": 10.0}), 5000) is None


def _run_all():
    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]
    for fn in fns:
        fn()
        print(f"  ok  {fn.__name__}")
    print(f"\n{len(fns)} passed")


if __name__ == "__main__":
    _run_all()
