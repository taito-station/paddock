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

from pred_header import (
    GOLDEN_PATH,
    HEADER,
    HEADER_NUM_VENUE,
    NoHeaderFound,
    split_by_header,
)

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


def test_shared_golden_is_parsed_with_the_expected_fields():
    # 生成側（Rust の race_heading）が同じファイルと突き合わせている（session.rs の
    # heading_samples_match_the_shared_golden）。片方だけ見出しを変えると、必ずどちらかが落ちる。
    # マッチするだけでなく **どの値がどのフィールドに入るか** まで見る（場と馬場が入れ替わっても
    # 「マッチはする」ので、regex 一致だけでは正しい組を保証できない）。
    with open(GOLDEN_PATH, encoding="utf-8") as f:
        lines = [ln for ln in f.read().splitlines() if ln.strip()]
    assert len(lines) == 5, lines
    expected = [
        ("1", "新潟", "芝", "2000"),
        ("5", "新潟", "芝", "2000"),
        ("8", "新潟", "芝", "2000"),
        ("9", "新潟", "芝", "2000"),
        ("1", "東京", "芝", "1600"),
    ]
    for ln, want in zip(lines, expected):
        assert re.match("^" + HEADER, ln).groups() == want, ln
        assert re.match("^" + HEADER_NUM_VENUE, ln).groups() == want[:2], ln


def test_split_by_header_raises_when_a_pred_table_has_no_header():
    # 無言死の直接の塞ぎ。確率テーブルなのに 0 件なら例外（＝呼び出し側は非 0 終了）。
    body = "   1 サンプルウマ  33.6%  40.0%  50.0%\n   2 テストホース  12.1%  20.0%  28.0%\n"
    try:
        split_by_header(body, HEADER, "dummy")
    except NoHeaderFound as e:
        assert "見出しが 1 件も見つかりません" in str(e), e
    else:
        raise AssertionError("見出し 0 件なのに落ちなかった")


def test_split_by_header_allows_empty_input():
    # 空入力は異常ではない（正当に 0 レース）。ここまで落とすと通常運用が壊れる。
    assert split_by_header("   \n", HEADER, "dummy") == ["   \n"]


def test_split_by_header_allows_the_no_meeting_message():
    # 開催の無い日の predict は 1 行だけ出す。これは正当な 0 レースなので落としてはいけない
    # （落とすと「見出し形式が変わった」と誤誘導することになる）。
    text = "この日の開催はありません: 2026-08-13\n"
    assert split_by_header(text, HEADER, "dummy") == [text]


def test_extract_preds_script_exits_when_header_is_unrecognized():
    # スクリプト側の同じ塞ぎ。空配列を吐いて exit 0 する挙動に戻ったらここで落ちる。
    fd, path = tempfile.mkstemp(suffix=".txt")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write("--- レース 1: 東京 芝 2000m ===\n   1 ウマ  10.0%  20.0%  30.0%\n")
        proc = subprocess.run(
            [sys.executable, os.path.join(HERE, "extract_preds.py"), path],
            capture_output=True,
            text=True,
        )
        assert proc.returncode != 0, proc.stdout
        assert "見出しが 1 件も見つかりません" in proc.stderr, proc.stderr
    finally:
        os.unlink(path)


def test_extract_preds_script_allows_the_no_meeting_message():
    # 開催の無い日は従来どおり [] / exit 0（スクリプト側も同じ規律）。
    fd, path = tempfile.mkstemp(suffix=".txt")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write("この日の開催はありません: 2026-08-13\n")
        proc = subprocess.run(
            [sys.executable, os.path.join(HERE, "extract_preds.py"), path],
            capture_output=True,
            text=True,
        )
        assert proc.returncode == 0, proc.stderr
        assert json.loads(proc.stdout) == [], proc.stdout
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
