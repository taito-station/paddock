"""gen_pure_preds.parse_table / calibration.load_pure の最小テスト（pytest 不要・`python3 test_gen_pure_preds.py`）.

計測ハーネスの根幹は `analyze predict` の表示テーブルをパースする ROW 正規表現。出力書式が
変わると無言で空パースに陥るため、馬行の抽出と非馬行の無視を assert で固定する。
"""
import os
import tempfile

import calibration as C
import gen_pure_preds as G


def approx(a, b, eps=1e-9):
    return abs(a - b) < eps


def test_parse_table_extracts_horse_rows():
    text = (
        "予想: 2026-3-kyoto-11-10R\n"
        "馬番 馬名 勝率 連対率 複勝率\n"          # ヘッダ（非馬行）→ 無視
        "   1 オリーブグリーン   1.5%   11.3%   16.1%\n"
        "  12 ロングネームの馬   13.8%   16.4%   28.2%\n"
        "\n"                                        # 空行 → 無視
    )
    rows = G.parse_table(text)
    assert [r[0] for r in rows] == [1, 12], rows  # 馬番の抽出と順序
    want = [(0.015, 0.113, 0.161), (0.138, 0.164, 0.282)]
    for (_, w, p, s), (ew, ep, es) in zip(rows, want):
        assert approx(w, ew) and approx(p, ep) and approx(s, es), rows


def test_parse_table_ignores_non_matching():
    # 末尾が % でも馬番(行頭数字)が無ければ拾わない
    assert G.parse_table("合計 100.0% 200.0% 300.0%\n") == []
    assert G.parse_table("見出しのみ\n") == []
    assert G.parse_table("") == []


def test_load_pure_roundtrip():
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "pure.tsv")
        with open(p, "w") as f:
            f.write("race_slug\tnk12\thorse_num\twin\tplace\tshow\n")
            f.write("2026-3-kyoto-11-10R\t202608031110\t1\t0.01500\t0.08300\t0.13000\n")
            f.write("2026-3-kyoto-11-10R\t202608031110\t2\t0.13800\t0.16400\t0.28200\n")
        preds, nk = C.load_pure(p)
        assert nk == {"2026-3-kyoto-11-10R": "202608031110"}, nk
        horses = preds["2026-3-kyoto-11-10R"]
        assert approx(horses[1][0], 0.015) and approx(horses[2][2], 0.282), horses


def test_load_pure_empty_file_no_crash():
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "empty.tsv")
        open(p, "w").close()  # 完全な空ファイル（ヘッダ行すら無い）
        preds, nk = C.load_pure(p)  # next(f, None) で StopIteration を出さない
        assert preds == {} and nk == {}


if __name__ == "__main__":
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print(f"ok {name}")
    print("all passed")
