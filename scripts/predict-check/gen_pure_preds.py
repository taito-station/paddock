#!/usr/bin/env python3
"""純モデル（α=1.0）の確率を bt_races の各レースで再生成する（校正計測の入力）。

`paddock-analyze predict <slug> --blend-alpha 1.0` を各レースで実行し、確率テーブル
（馬番/馬名/勝率/連対率/複勝率）をパースして TSV に落とす。市場ブレンドを切った
純モデル出力なので、win/place/show の校正（reliability / ECE）計測に使える。

α の意味（重要）: 実装は `blended = α·model + (1−α)·market`（estimate.rs / ADR 0045・0052）で
**α＝モデル重み**。**α=1.0 が純モデル / α=0.0 が純市場 / α=0.2 が現行**。以前の docstring・既定値は
「α=0=純モデル」と逆だった（#303 で是正）。純モデル校正を測るには α=1.0 を渡す（既定もこれに変更）。

出力 TSV 列: race_slug, nk12, horse_num, win, place, show（win/place/show は小数 0..1）

注意1: `analyze predict` は production 設定を通すため、place/show は本番採用の冪変換
（place_show_power, ADR 0047）適用済みの値になる。本番が実際に表示する純モデル校正を
そのまま測る用途。素スコア段階の校正を測りたい場合は place_show_power を off にした
ビルド/設定で生成すること。
注意2: `analyze predict` は集計統計に as_of=None（全期間=未来込み）を使うため、過去レースを
再予想すると未来データがリークする（#302）。本スクリプトはその制約を引き継ぐ。リーク無しの
集約スコアは `analyze backtest` を使う（#302 / ADR 0052）。

使い方:
    python3 scripts/predict-check/gen_pure_preds.py \
        --races /tmp/bt252/bt_races.tsv --bin ./target/debug/paddock-analyze \
        --out /tmp/bt252/pure_preds.tsv
"""
import argparse
import os
import re
import subprocess
import sys

# "   1 オリーブグリーン   1.5%   11.3%   16.1%" → (num, win, place, show)
# 馬名は捨てるので非捕捉。捕捉群は 馬番 / win / place / show の 4 つ。
ROW = re.compile(r"^\s*(\d+)\s+(?:\S.*?)\s+([\d.]+)%\s+([\d.]+)%\s+([\d.]+)%\s*$")


def parse_table(text):
    rows = []
    for line in text.splitlines():
        m = ROW.match(line)
        if m:
            num = int(m.group(1))
            win, place, show = (float(m.group(i)) / 100.0 for i in (2, 3, 4))
            rows.append((num, win, place, show))
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--races", default="/tmp/bt252/bt_races.tsv")
    ap.add_argument("--bin", default="./target/debug/paddock-analyze")
    ap.add_argument("--out", default="/tmp/bt252/pure_preds.tsv")
    ap.add_argument("--alpha", default="1.0",
                    help="blend-alpha＝モデル重み（既定 1.0=純モデル。α=0 は純市場、α=0.2 が現行）")
    args = ap.parse_args()

    # 出力 TSV は純モデル前提（calibration.py も純モデル＝α=1.0 として集計）。α≠1.0 を渡すと
    # 「純モデルでない値」が pure_preds として下流に流れるので警告する。
    try:
        alpha_is_pure_model = float(args.alpha) == 1.0  # "1" / "1.0" / "1.00" などを等価に扱う
    except ValueError:
        alpha_is_pure_model = False  # 数値でなければ predict 側に委ねつつ警告
    if not alpha_is_pure_model:
        print(f"WARN: --alpha={args.alpha}（≠1.0）。出力は純モデルではない（下流は純モデル＝α=1.0 前提）",
              file=sys.stderr)

    # host は localhost を避け 127.0.0.1 を使う（兄弟スクリプトと同じ。localhost だと間欠失敗が再発）。
    db = os.environ.get("PADDOCK_DB_URL", "postgres://paddock:paddock@127.0.0.1:5432/paddock")
    env = {**os.environ, "PADDOCK_DB_URL": db}

    races = []
    with open(args.races) as f:
        for line in f:
            parts = line.rstrip("\n").split("\t")
            if len(parts) >= 7:
                # parts[1] = paddock のレース id（slug 状, predict が受ける引数）, parts[6] = netkeiba 12 桁
                races.append((parts[1], parts[6]))  # slug, nk12

    n_ok = n_failed = 0
    with open(args.out, "w") as out:
        out.write("race_slug\tnk12\thorse_num\twin\tplace\tshow\n")
        for slug, nk in races:
            try:
                res = subprocess.run(
                    [args.bin, "predict", slug, "--blend-alpha", args.alpha],
                    env=env, capture_output=True, text=True, timeout=120,
                )
            except subprocess.TimeoutExpired:
                n_failed += 1  # timeout も failed として件数に含める
                print(f"TIMEOUT {slug}", file=sys.stderr)
                continue
            rows = parse_table(res.stdout)
            if not rows:
                n_failed += 1
                print(f"EMPTY {slug} (rc={res.returncode})", file=sys.stderr)
                continue
            n_ok += 1
            for num, win, place, show in rows:
                out.write(f"{slug}\t{nk}\t{num}\t{win:.5f}\t{place:.5f}\t{show:.5f}\n")
    print(f"done: {n_ok} races written, {n_failed} empty/failed → {args.out}")


if __name__ == "__main__":
    main()
