"""netkeiba の未発売番兵値をオッズとして採用しないためのガード（#621・#630）。

netkeiba は「未発売・該当なしの組み合わせ」に `99999.9` のような固定値を入れる。これは
**払戻倍率ではない**のに、EV は `的中確率 × オッズ` で作られるため 1 点で EV が 3 桁になり、
ポートフォリオの参考 ROI が跳ね上がる（実測: 三連複 1 点で ROI 612.6%）。

Rust 側は `OddsValue::try_from`（`src/domain/src/odds/odds_value.rs`）が同じ番兵を弾くが、
**この scripts/ 配下は psql / TSV で DB を直読みするので、そのガードを一切通らない**。
分析（`live_ev` / `gate_calibration` / `snapshot_ev_report` / `umaren_backtest`）が汚染された
ROI を出さないよう、オッズを float 化する入口でここを通す。

**番兵は券種別**（#630/#634）。ワイドの番兵は `9999.9` だが、三連複・三連単には**正当な**
`9999.9` の配当が実在する（9000〜11000 帯に trio 6,244 行・trifecta 56,230 行=2026-08-18 実測）。単勝・複勝に
番兵は無い（DB 実測 0 行）。だから `is_sentinel` / `is_payout_odds` は**券種を必須第 1 引数**に
取る——既定値を持たせると呼び出し側の更新漏れが静かに通り、#621 と同じ「例外を出さない
壊れ方」を再生産する。未知の券種ラベルは `ValueError`。

**番兵リストの正本は Rust 側と共有**（`src/domain/src/odds/netkeiba_sentinels.txt`。TAB 区切り
`券種<TAB>値` の 2 列で、番兵を持たない券種は行そのものを置かない）。同じ値を両言語が別々に
持つと片方だけ更新して静かにズレるため、このモジュールはそのファイルを読む。Rust 側は同じ
ファイルを `include_str!` してテストで突き合わせている。

**上限方式を採らない理由**: 三連単には `111971.9` / `200886.6` のような正当な高配当が実在する
（DB 実測）。上限は大穴を殺すが、番兵は券種ごとの固定値なので特定値の除外なら誤爆しない。
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

# Rust の BetType（Display=snake_case）と同じ 7 ラベル。正本ファイルの検証と、公開 API の
# 未知ラベル検出（ValueError）の両方に使う。win / place は番兵を持たないが**ラベルとしては
# 有効**——「win に番兵は無い」と「win という券種は無い」を混同しないこと。
VALID_BET_TYPES = ("win", "place", "quinella", "wide", "exacta", "trio", "trifecta")


def _load_sentinels():
    """正本ファイルを読んで {券種ラベル: 番兵値タプル} を返す。

    **失敗しても空 dict にフォールバックしない。** 番兵リストが空になると
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

    sentinels = {}
    for lineno, raw in enumerate(lines, start=1):
        line = raw.strip()
        if not line:
            continue
        parts = line.split("\t")
        if len(parts) != 2:
            raise RuntimeError(
                f"番兵リストは `券種<TAB>値` の 2 列のみ: {SENTINELS_PATH}:{lineno} が {line!r}。"
                "コメント行・区切り・ヘッダは書けない（Rust 側の golden も同じ書式を要求する）"
            )
        label, text = parts[0].strip(), parts[1].strip()
        if label not in VALID_BET_TYPES:
            raise RuntimeError(
                f"番兵リストに未知の券種ラベル: {SENTINELS_PATH}:{lineno} が {label!r}。"
                f"ラベルは Rust の BetType(snake_case) と揃える（有効: {', '.join(VALID_BET_TYPES)}）"
            )
        try:
            value = float(text)
        except ValueError as e:
            raise RuntimeError(
                f"番兵リストの値が数値でない: {SENTINELS_PATH}:{lineno} が {text!r}。"
                "各行は `券種<TAB>値` で値は f64（Rust 側の golden も同じ書式を要求する）"
            ) from e
        # float() は nan / inf / 1e400 を通す。非有限値は番兵として登録しても
        # abs(o - nan) < eps が常に偽で**その番兵だけが無言で無効化**される
        # ——空リストを拒否するのと同じ理由でここも受理しない。
        if not math.isfinite(value):
            raise RuntimeError(
                f"番兵リストに非有限値: {SENTINELS_PATH}:{lineno} が {text!r}。"
                "非有限の番兵は比較が常に偽になり、その値だけが黙って無効化される"
            )
        sentinels.setdefault(label, []).append(value)

    if not sentinels:
        raise RuntimeError(
            f"番兵リストが空: {SENTINELS_PATH}。"
            "空だと番兵が素通りして EV が汚染されるため、空を正常として受理しない"
        )
    return {label: tuple(values) for label, values in sentinels.items()}


NETKEIBA_SENTINELS = _load_sentinels()


def _sentinels_for(bet_type):
    """券種ラベルを検証し、その券種の番兵タプル（win/place は空）を返す。

    未知ラベルは **ValueError で止める**。False に畳むと「ラベルの typo で番兵が素通り」が
    #621 と同じ静かな壊れ方になる。オッズ値の float 化より先に呼ぶこと（値が数値かどうかに
    依らずラベルの誤りを検出する）。
    """
    if bet_type not in VALID_BET_TYPES:
        raise ValueError(
            f"未知の券種ラベル: {bet_type!r}（有効: {', '.join(VALID_BET_TYPES)}）"
        )
    return NETKEIBA_SENTINELS.get(bet_type, ())


def is_sentinel(bet_type, odds):
    """その券種における netkeiba の未発売番兵値か（払戻倍率として使ってはいけない値か）。

    券種は必須（#630）。同じ `9999.9` でもワイドなら番兵・三連複なら正当な配当。
    """
    sentinels = _sentinels_for(bet_type)
    try:
        o = float(odds)
    except (TypeError, ValueError):
        return False
    return any(abs(o - s) < SENTINEL_EPSILON for s in sentinels)


def is_payout_odds(bet_type, odds):
    """その券種の払戻倍率として採用してよい値か（有限・1.0 以上・その券種の番兵でない）。

    Rust の `OddsValue::try_from((BetType, f64))` と同じ判定。ここを通らなかった組み合わせは
    「オッズ不明」として扱う——0 円として計上するのではなく、**その脚を落とす**こと
    （0 扱いだと ROI の分母には残り、実際より悪い数字になる）。
    """
    sentinels = _sentinels_for(bet_type)
    try:
        o = float(odds)
    except (TypeError, ValueError):
        return False
    if not math.isfinite(o):
        return False
    return o >= 1.0 and not any(abs(o - s) < SENTINEL_EPSILON for s in sentinels)
