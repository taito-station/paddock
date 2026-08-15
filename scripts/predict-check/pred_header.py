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

import re
import sys

# 場・馬場・距離まで取る版（`re.split` の stride は 5）。
HEADER = r"--- レース (\d+): (\S+) (\S+) (\d+)m[^\n]*---"

# レース番号と場だけ取る版（`re.split` の stride は 3）。馬場・距離は捨てる。
HEADER_NUM_VENUE = r"--- レース (\d+): (\S+) \S+ \d+m[^\n]*---"

# 生成側（Rust の `race_heading`）と解析側が同じ見出しを見ていることを固定する golden。
# 生成側は `src/apps/predict/src/session.rs` が `include_str!` で読む。
GOLDEN_PATH = "testdata/pred_header_samples.txt"


def split_by_header(text, pattern, source):
    """見出しで分割する。**中身があるのに 1 件も取れなければ即座に落とす**（#587）。

    このパースの本当の危険は regex が古くなることそのものではなく、壊れても
    例外が出ず「0 レース」として静かに通ることにある（回収率の検証が空のまま回る）。
    `re.split` は見出しが 1 つも無いと要素 1 個のリストを返すので、そこを異常として扱う。

    `source` はエラー文言に出す入力の説明（ファイルパス等）。
    """
    blocks = re.split(pattern, text)
    if len(blocks) == 1 and text.strip():
        sys.exit(
            f"[pred_header] {source}: レース見出しが 1 件も見つかりません。"
            "入力が predict の確率テーブルか、見出し形式が変わっていないか確認してください"
            f"（期待する形: {GOLDEN_PATH} 参照）。"
        )
    return blocks
