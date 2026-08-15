#!/usr/bin/env python3
"""見出し解析契約（pred_header）の回帰テスト（#587）。

このパースの壊れ方は **例外ではなく「0 件」** なので、実データを流すまで誰も気づかない
（#587 で見出し末尾に「（発走 HH:MM）」「[発走済]」が付いたとき、6 スクリプトが同時に
無言で空を返した）。旧形式・発走時刻付き・発走時刻不明の 3 形式をここで固定する。

発走時刻不明の `--:--` は**ハイフンを含む**ため、末尾を素朴に切る regex（`[^-]*` 等）だと
これだけ落ちる。3 形式を必ず一緒に見ること。
"""

import json
import os
import re
import subprocess
import sys
import tempfile

from pred_header import HEADER, HEADER_NUM_VENUE

HERE = os.path.dirname(os.path.abspath(__file__))

OLD = "--- レース 1: 東京 芝 2000m ---"
WITH_POST = "--- レース 2: 新潟 ダート 1200m（発走 09:40）---"
STARTED_UNKNOWN_POST = "--- レース 3: 中京 芝 1600m（発走 --:--）[発走済] ---"
ALL_FORMS = (OLD, WITH_POST, STARTED_UNKNOWN_POST)


def test_header_extracts_venue_surface_distance_in_all_forms():
    pat = re.compile("^" + HEADER)
    assert pat.match(OLD).groups() == ("1", "東京", "芝", "2000")
    assert pat.match(WITH_POST).groups() == ("2", "新潟", "ダート", "1200")
    assert pat.match(STARTED_UNKNOWN_POST).groups() == ("3", "中京", "芝", "1600")


def test_header_num_venue_extracts_race_num_and_venue_in_all_forms():
    pat = re.compile("^" + HEADER_NUM_VENUE)
    assert pat.match(OLD).groups() == ("1", "東京")
    assert pat.match(WITH_POST).groups() == ("2", "新潟")
    assert pat.match(STARTED_UNKNOWN_POST).groups() == ("3", "中京")


def test_split_stride_is_independent_of_header_form():
    # re.split の stride（HEADER=5 / HEADER_NUM_VENUE=3）は下流のインデックス計算の前提。
    # 末尾の付加情報を非キャプチャで受けているので、形式が変わっても動いてはいけない。
    text = "".join(f"{h}\n   1 ウマ  10.0%  20.0%  30.0%\n" for h in ALL_FORMS)
    assert len(re.split(HEADER, text)) == 1 + 5 * len(ALL_FORMS)
    assert len(re.split(HEADER_NUM_VENUE, text)) == 1 + 3 * len(ALL_FORMS)


def test_greedy_tail_does_not_swallow_the_next_header():
    # `[^\n]*` は行内で貪欲だが、見出しは 1 行なので次の見出しまで飲み込まない
    # （飲み込むとレース数が減り、これも「静かに減る」壊れ方になる）。
    text = "".join(f"{h}\n" for h in ALL_FORMS)
    assert len(re.findall(HEADER, text)) == len(ALL_FORMS)


def test_extract_preds_script_reads_every_form():
    # 主パーサは関数ではなくスクリプト（README の手順 3 が stdout をリダイレクトして渡す）。
    # regex 単体ではなく実際に起動して、3 形式すべてが 1 レースずつ取れることを見る。
    body = "".join(
        f"{h}\n   1 サンプルウマ  33.6%  40.0%  50.0%\n   2 テストホース  12.1%  20.0%  28.0%\n\n"
        for h in ALL_FORMS
    )
    fd, path = tempfile.mkstemp(suffix=".txt")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(body)
        proc = subprocess.run(
            [sys.executable, os.path.join(HERE, "extract_preds.py"), path],
            capture_output=True,
            text=True,
        )
        assert proc.returncode == 0, proc.stderr
        races = json.loads(proc.stdout)
        assert len(races) == len(ALL_FORMS), races
        assert [r["race_num"] for r in races] == [1, 2, 3], races
        assert [r["venue"] for r in races] == ["東京", "新潟", "中京"], races
        assert all(len(r["horses"]) == 2 for r in races), races
    finally:
        os.unlink(path)


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")


if __name__ == "__main__":
    main()
