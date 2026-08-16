"""netkeiba の未発売番兵値をオッズとして採用しないためのガード（#621）。

netkeiba は「未発売・該当なしの組み合わせ」に `99999.9` のような固定値を入れる。これは
**払戻倍率ではない**のに、EV は `的中確率 × オッズ` で作られるため 1 点で EV が 3 桁になり、
ポートフォリオの参考 ROI が跳ね上がる（実測: 三連複 1 点で ROI 612.6%）。

Rust 側は `OddsValue::try_from`（`src/domain/src/odds/odds_value.rs`）が同じ番兵を弾くが、
**この scripts/ 配下は psql / TSV で DB を直読みするので、そのガードを一切通らない**。
分析（`live_ev` / `gate_calibration` / `snapshot_ev_report` / `umaren_backtest`）が汚染された
ROI を出さないよう、オッズを float 化する入口でここを通す。

**番兵リストの正本は Rust 側と共有**（`src/domain/src/odds/netkeiba_sentinels.txt`）。
同じ値を両言語が別々に持つと片方だけ更新して静かにズレるため、このモジュールはそのファイルを
読む。Rust 側は同じファイルを `include_str!` してテストで突き合わせている。

**上限方式を採らない理由**: 三連単には `111971.9` / `200886.6` のような正当な高配当が実在する
（DB 実測）。上限は大穴を殺すが、番兵は固定値なので特定値の除外なら誤爆しない。
"""

import math
import os

# 番兵リストの正本（Rust の NETKEIBA_SENTINELS と同一ファイル）。
SENTINELS_PATH = os.path.normpath(
    os.path.join(
        os.path.dirname(os.path.abspath(__file__)),
        "..",
        "..",
        "src/domain/src/odds/netkeiba_sentinels.txt",
    )
)

# DB の double precision を往復しても取りこぼさない幅（Rust の SENTINEL_EPSILON と同値）。
SENTINEL_EPSILON = 1e-6


def _load_sentinels():
    """正本ファイルを読んで番兵値のタプルを返す。

    **失敗しても空タプルにフォールバックしない。** 番兵リストが空になると
    `is_payout_odds` が番兵を素通しし、汚染された EV / 参考 ROI を「正常な出力」として
    出してしまう（#621 の実害そのものが黙って戻る）。読めないなら**原因を示して止める**。
    """
    # UnicodeDecodeError は ValueError のサブクラスで OSError ではない。別文字コードで
    # 保存し直された正本を OSError だけで受けると、例外の str() にパスが載らず
    # 「原因不明で解析スクリプトが全部起動不能」がそのまま残る。
    try:
        with open(SENTINELS_PATH, encoding="utf-8") as f:
            lines = f.readlines()
    except (OSError, UnicodeDecodeError) as e:
        raise RuntimeError(
            f"番兵リストの正本を読めない: {SENTINELS_PATH} ({e})。"
            "Rust と共有する本番依存ファイルなので、消さずに UTF-8 で復元すること"
            "（docs/specifications/netkeiba-datasource.md の番兵の節）"
        ) from e

    sentinels = []
    for lineno, raw in enumerate(lines, start=1):
        line = raw.strip()
        if not line:
            continue
        try:
            value = float(line)
        except ValueError as e:
            raise RuntimeError(
                f"番兵リストは 1 行 1 値の数値のみ: {SENTINELS_PATH}:{lineno} が {line!r}。"
                "コメント行・区切り・ヘッダは書けない（Rust 側の golden も同じ書式を要求する）"
            ) from e
        # float() は nan / inf / 1e400 を通す。非有限値は番兵として登録しても
        # abs(o - nan) < eps が常に偽で**その番兵だけが無言で無効化**される
        # ——空リストを拒否するのと同じ理由でここも受理しない。
        if not math.isfinite(value):
            raise RuntimeError(
                f"番兵リストに非有限値: {SENTINELS_PATH}:{lineno} が {line!r}。"
                "非有限の番兵は比較が常に偽になり、その値だけが黙って無効化される"
            )
        sentinels.append(value)

    if not sentinels:
        raise RuntimeError(
            f"番兵リストが空: {SENTINELS_PATH}。"
            "空だと番兵が素通りして EV が汚染されるため、空を正常として受理しない"
        )
    return tuple(sentinels)


NETKEIBA_SENTINELS = _load_sentinels()


def _is_sentinel_float(o):
    """float 化済みの値が番兵か。二重変換を避けるための内部関数。"""
    return any(abs(o - s) < SENTINEL_EPSILON for s in NETKEIBA_SENTINELS)


def is_sentinel(odds):
    """netkeiba の未発売番兵値か（払戻倍率として使ってはいけない値か）。"""
    try:
        return _is_sentinel_float(float(odds))
    except (TypeError, ValueError):
        return False


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
    if not math.isfinite(o):
        return False
    return o >= 1.0 and not _is_sentinel_float(o)
