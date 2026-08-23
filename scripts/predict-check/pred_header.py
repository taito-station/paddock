"""predict の確率テーブル見出し行を読む正規表現と、その解析ヘルパ（#587）。

`paddock-predict` の stdout を機械パースする 6 スクリプト（extract_preds / live_ev /
win_backtest / umaren_backtest / konsen_backtest / formation_backtest）が同じ見出し契約を
持つので、regex をここに 1 本化する。

**なぜ共有するか**: 元は 6 か所に同じ regex が複製されており、#587 で見出し末尾に
「（発走 HH:MM）」「[発走済]」が付いた際に 6 本が同時に壊れた。しかも**壊れ方が例外ではなく
「0 件」**（`extract_preds` なら空配列 + stderr に `# 0 races`）なので、回収率の検証が黙って
空になる。複製が残る限り、次に見出しを変えたときも同じ追従漏れが起こる。

**契約**: 距離 `\\d+m` の後ろは末尾の `---` まで緩く受ける。旧形式（`... 2000m ---`）も
新形式（`... 2000m（発走 09:40）[発走済] ---`）も通る。発走時刻不明の `--:--` はハイフンを
含むので、末尾を素朴に切る（`[^-]*` 等）と壊れる点に注意。
"""

import os
import re

# 場・馬場・距離まで取る版（`re.split` の stride は 5）。
HEADER = r"--- レース (\d+): (\S+) (\S+) (\d+)m[^\n]*---"

# レース番号と場だけ取る版（`re.split` の stride は 3）。馬場・距離は捨てる。
HEADER_NUM_VENUE = r"--- レース (\d+): (\S+) \S+ \d+m[^\n]*---"

# 確率テーブルの馬行（`  3 ウマ 12.3% ...`）。**入力の種別判定にだけ使う**緩い版で、
# 各スクリプトが持つ厳密な行 regex を置き換えるものではない。
RACE_ROW = re.compile(r"^\s*\d+\s+\S+\s+[\d.]+%", re.MULTILINE)

# 生成側（Rust `race_heading` / session.rs）と解析側（Python）が同じ見出しを見て
# いることを固定する golden（#587）。`include_str!` は crate 外を参照できないため
# ファイルは predict crate 内に置く（ADR 0085）。変更時は `race_heading` /
# `pred_header_samples.txt` / `test_pred_header.py` を同じ PR で触ること。
# 行セマンティクスは docs/specifications/predict-session.md。
GOLDEN_DISPLAY = "src/apps/predict/testdata/pred_header_samples.txt"
GOLDEN_PATH = os.path.normpath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", GOLDEN_DISPLAY)
)


class NoHeaderFound(Exception):
    """確率テーブルらしき入力なのに見出しが 1 件も取れなかった（#587）。

    ライブラリ側では投げるだけにして、終了コードへの変換は各スクリプトの入口が行う
    （共有ヘルパが `sys.exit` を直に呼ぶと、別文脈から使うとき制御できなくなる）。
    """

    def __init__(self, source):
        super().__init__(no_header_message(source))


def no_header_message(source):
    """見出し 0 件のエラー文言（#587）。文言を 2 か所に複製しないための単一ソース。"""
    return (
        f"[pred_header] {source}: レース見出しが 1 件も見つかりません。"
        "入力が predict の確率テーブルか、見出し形式が変わっていないか確認してください"
        f"（期待する形: {GOLDEN_DISPLAY} 参照）。"
    )


def looks_like_pred_table(text):
    """確率テーブルらしい入力か（馬行が 1 行でもあるか）。

    開催の無い日の `paddock-predict` は「この日の開催はありません: <date>」の 1 行だけを出す。
    これは**正当な 0 レース**なので、見出しが無いことを異常にしてはいけない。逆に馬行があるのに
    見出しが取れないのは、入力違いか見出し形式の変化（#587 の無言死）。

    **これは heuristic**。馬行の書式まで同時に変わった入力では偽陰性になり、元の
    「静かに 0 件」に戻る。見出しと馬行が同時に変わる改修では、このガードを当てにしないこと。
    """
    return bool(RACE_ROW.search(text))


def ensure_header_found(text, found, source):
    """見出しが 1 件も取れていない異常を検出する共通判定（#587）。

    判定そのものを 1 か所に置く（`split_by_header` を通せない `extract_preds` と条件が
    ズレると、片方だけ静かに素通りする——この PR が潰した複製と同じ構図になる）。

    **このガードが捕まえるのは「全滅」だけ**。一部のレースの見出しだけが変わった場合は
    その分だけ静かに減る、という同じクラスの故障が残る（`--- レース ` 行の総数と突き合わせる
    案もあるが、行の書式自体が契約なので二重の脆さになる）。
    """
    if found == 0 and looks_like_pred_table(text):
        raise NoHeaderFound(source)


def split_by_header(text, pattern, source):
    """見出しで分割する。**確率テーブルなのに 1 件も取れなければ例外**（#587）。

    このパースの本当の危険は regex が古くなることそのものではなく、壊れても例外が出ず
    「0 レース」として静かに通ることにある（回収率の検証が空のまま回る）。
    `re.split` は見出しが 1 つも無いと要素 1 個のリストを返すので、そこを判定に使う。
    """
    blocks = re.split(pattern, text)
    ensure_header_found(text, len(blocks) - 1, source)
    return blocks
