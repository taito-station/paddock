# testdata — 言語をまたぐ契約の golden

## `pred_header_samples.txt`

`paddock-predict` が出すレース見出しの **golden**（#587）。生成側（Rust）と解析側（Python）が
**同じファイル**を見ることで、片方だけ変えたときに必ずどちらかのテストが落ちるようにする。

- 生成側: `src/apps/predict/src/session.rs` の `heading_samples_match_the_shared_golden`
  （`include_str!` で読むので、ファイルが消えるとコンパイルが通らない）
- 解析側: `scripts/predict-check/test_pred_header.py` の
  `test_shared_golden_is_parsed_by_both_patterns`

**なぜ必要か**: 見出しの解析は壊れても例外を出さず「0 件」になる（無言死）。regex を
`pred_header.py` に 1 本化しても、それは Python 内の複製を消すだけで、**Rust の出力と Python の
期待値がリテラルの一致頼み**という言語境界の複製は残る。ここが唯一の突き合わせ点。

### 行の意味（順序に依存するので並べ替えない）

1. 発走時刻あり・発走済み（`[発走済]`）
2. 発走時刻あり・未発走
3. 発走時刻不明（`--:--`）— **ハイフンを含む**ので、末尾を素朴に切る regex だとここだけ落ちる
4. **旧形式**（#587 以前）— Rust はもう生成しないが、archived な `bt_pred_*.txt` と
   `gen_win_backtest_data.sh` の生成分が今もこの形。後方互換を落とさないための行

行を足すときは、Rust 側（1〜3 の生成物と一致）と Python 側（全行がパースできる）の
両方のテストを同時に更新すること。
