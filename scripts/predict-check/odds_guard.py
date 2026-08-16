"""netkeiba の未発売番兵値をオッズとして採用しないためのガード（#621）。

netkeiba は「未発売・該当なしの組み合わせ」に `99999.9` のような固定値を入れる。これは
**払戻倍率ではない**のに、EV は `的中確率 × オッズ` で作られるため 1 点で EV が 3 桁になり、
ポートフォリオの参考 ROI が跳ね上がる（実測: 三連複 1 点で ROI 612.6%）。

Rust 側は `OddsValue::try_from`（`src/domain/src/odds/odds_value.rs`）が同じ番兵を弾くが、
**この scripts/ 配下は psql / TSV で DB を直読みするので、そのガードを一切通らない**。
分析（`live_ev` / `gate_calibration` / `snapshot_ev_report` / `umaren_backtest`）が汚染された
ROI を出さないよう、オッズを float 化する入口でここを通す。

**番兵リストの正本は Rust 側と共有**（`src/domain/src/odds/testdata/netkeiba_sentinels.txt`）。
同じ値を両言語が別々に持つと片方だけ更新して静かにズレるため、このモジュールはそのファイルを
読む。Rust 側は同じファイルを `include_str!` してテストで突き合わせている。

**上限方式を採らない理由**: 三連単には `111971.9` / `200886.6` のような正当な高配当が実在する
（DB 実測）。上限は大穴を殺すが、番兵は固定値なので特定値の除外なら誤爆しない。
"""

import os

# 番兵リストの正本（Rust の NETKEIBA_SENTINELS と同一ファイル）。
SENTINELS_PATH = os.path.normpath(
    os.path.join(
        os.path.dirname(os.path.abspath(__file__)),
        "..",
        "..",
        "src/domain/src/odds/testdata/netkeiba_sentinels.txt",
    )
)

# DB の double precision を往復しても取りこぼさない幅（Rust の SENTINEL_EPSILON と同値）。
SENTINEL_EPSILON = 1e-6


def _load_sentinels():
    with open(SENTINELS_PATH, encoding="utf-8") as f:
        return tuple(float(line) for line in f if line.strip())


NETKEIBA_SENTINELS = _load_sentinels()


def is_sentinel(odds):
    """netkeiba の未発売番兵値か（払戻倍率として使ってはいけない値か）。"""
    try:
        o = float(odds)
    except (TypeError, ValueError):
        return False
    return any(abs(o - s) < SENTINEL_EPSILON for s in NETKEIBA_SENTINELS)


def is_payout_odds(odds):
    """払戻倍率として採用してよい値か（有限・1.0 以上・番兵でない）。

    Rust の `OddsValue::try_from` と同じ判定。ここを通らなかった組み合わせは
    「オッズ不明」として扱う——0 円として計上するのではなく、**その脚を落とす**こと
    （0 扱いだと ROI の分母には残り、実際より悪い数字になる）。
    """
    try:
        o = float(odds)
    except (TypeError, ValueError):
        return False
    if not (o == o) or o in (float("inf"), float("-inf")):  # NaN / ±inf
        return False
    return o >= 1.0 and not is_sentinel(o)
