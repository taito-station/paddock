#!/usr/bin/env python3
"""nk.parse_result の走査範囲（#766）のユニットテスト（自走式・stdlib のみ）。

固定する不変量:
(1) 行は結果表 `table#All_Result_Table` の中だけから取る（本体 Rust parse_race_result と同じ規則）。
    結果表の後ろのラップ表・走行距離表（RapSummary_Table / LapSummary_Table）にも `HorseList` 行があり、
    拾うと 1 着馬の馬番で着順なし・馬名なしの行が混ざって、後勝ちの辞書で 1 着馬の着順が潰れる。
(2) 結果表の中の取消・除外馬（着順なし・名前あり）の行は返す。
(3) 結果表が無いページは 0 行で、構造変化の疑いを warn する。`data-id=` などは結果表の id と見なさない。
"""

import contextlib
import io
import sys

import nk

RID = "202602011210"

RESULT_TABLE = (
    '<table summary="全着順" class="RaceTable01 RaceCommon_Table ResultRefund Table_Show_All"\n'
    'id="All_Result_Table">'
    '<tr class="HorseList"><td class="Result_Num"><div class="Rank">1</div></td>'
    '<td class="Num Waku6"><div>6</div></td><td class="Num Txt_C"><div>9</div></td>'
    '<span class="HorseNameSpan">シーズザスローン</span></tr>'
    '<tr class="HorseList"><td class="Result_Num"><div class="Rank">2</div></td>'
    '<td class="Num Waku7"><div>7</div></td><td class="Num Txt_C"><div>12</div></td>'
    '<span class="HorseNameSpan">ベータ</span></tr>'
    # 取消（着順なし・名前あり）。実ページと同じく class 属性が重複した行
    '<tr  class="Torikeshi HorseList" class="HorseList"><td class="Result_Num"><div class="Rank">取消</div></td>'
    '<td class="Num Waku1"><div>1</div></td><td class="Num Txt_C"><div>2</div></td>'
    '<span class="HorseNameSpan">ガンマ</span></tr>'
    "</table>"
)


# 実ページで結果表の後ろに出るラップ表・走行距離表（1 着馬の行。`class="Rank"` のセルと HorseNameSpan が無い）
def _after_table(cls, attrs):
    return (
        f'<table class="{cls}" {attrs}><tbody>'
        '<tr class="HorseList"><td class="Horse_Check Sticky"></td>'
        '<td class="Result_Num Sticky">1</td><td class="Num Waku6 Sticky"><div>9</div></td>'
        '<td class="Horse_Info Horse_Link"><a href="https://db.netkeiba.com/horse/2023104414">シーズザスローン</a></td>'
        '<td class="PitchCell">-</td></tr>'
        "</tbody></table>"
    )


AFTER_TABLES = (
    _after_table("RapSummary_Table", 'sumarry="走行距離表" id="milage_summary"')
    + _after_table("RapSummary_Table", 'summary="各馬ラップ表" id="lap_summary"')
    + _after_table("LapSummary_Table", 'sumarry="各馬ラップ表" id="LapSummary"')
)


def _parse(html, warn=False):
    err = io.StringIO()
    with contextlib.redirect_stderr(err):
        rows = nk.parse_result(html, RID, warn=warn)
    return rows, err.getvalue()


def test_rows_only_from_result_table():
    rows, _ = _parse(f"<html><body>{RESULT_TABLE}{AFTER_TABLES}</body></html>")
    assert rows == [
        {"rank": 1, "horse_num": 9, "name": "シーズザスローン"},
        {"rank": 2, "horse_num": 12, "name": "ベータ"},
        {"rank": None, "horse_num": 2, "name": "ガンマ"},
    ], rows
    # 後勝ちの辞書にしても 1 着馬の着順が潰れない（#724 で軸の勝率が 33%→4% に化けた形）
    assert {r["horse_num"]: r["rank"] for r in rows}[9] == 1


def test_scratched_row_inside_result_table_is_kept():
    rows, _ = _parse(RESULT_TABLE)
    assert [r["name"] for r in rows if r["rank"] is None] == ["ガンマ"]


def test_no_result_table_returns_no_rows_and_warns():
    rows, err = _parse(f"<html><body>{AFTER_TABLES}</body></html>", warn=True)
    assert rows == []
    assert "結果行を抽出できませんでした" in err


def test_data_id_attribute_is_not_the_result_table():
    html = RESULT_TABLE.replace('\nid="All_Result_Table"', ' data-id="All_Result_Table"')
    rows, err = _parse(html, warn=True)
    assert rows == [] and "結果行を抽出できませんでした" in err, rows


def main() -> int:
    fails = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            try:
                fn()
                print(f"ok   {name}")
            except AssertionError as e:
                fails += 1
                print(f"FAIL {name}: {e}")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
