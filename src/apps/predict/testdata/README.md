# testdata — 言語をまたぐ契約の golden

## `pred_header_samples.txt`

`paddock-predict` が出すレース見出しの **golden**（#587）。生成側（Rust）と解析側（Python）が
**同じファイル**を見ることで、片方だけ変えたときに必ずどちらかのテストが落ちるようにする。

- 生成側: `src/apps/predict/src/session.rs` の `heading_samples_match_the_shared_golden`
  （`include_str!` で読むので、ファイルが消えるとコンパイルが通らない）
- 解析側: `scripts/predict-check/test_pred_header.py` の
  `test_shared_golden_is_parsed_with_the_expected_fields`（マッチだけでなく、どの値がどの
  フィールドに入るかまで見る）

**置き場所が predict crate 側なのは意図的**。`include_str!` が crate の外を指すと
sparse checkout / パッケージングでテストがコンパイルできなくなる。Python 側はリポジトリ内の
相対参照で読めばよいので、制約の厳しい Rust 側に寄せている。

**なぜ必要か**: 見出しの解析は壊れても例外を出さず「0 件」になる（無言死）。regex を
`pred_header.py` に 1 本化しても、それは Python 内の複製を消すだけで、**Rust の出力と Python の
期待値がリテラルの一致頼み**という言語境界の複製は残る。ここが唯一の突き合わせ点。

### 行の意味（順序に依存するので並べ替えない）

1. 発走時刻あり・発走済み（`[発走済]`）
2. 発走時刻あり・未発走
3. 発走時刻不明（`--:--`）— **ハイフンを含む**ので、末尾を素朴に切る regex だとここだけ落ちる
4. 発走時刻不明 × 発走済み — 過去日の見返しで card に post_time が無いときの通常形。
   ハイフンとマークが同時に出る、解析側が最も落としやすい組み合わせ
5. **旧形式**（#587 以前）— Rust はもう生成しないが、archived な `bt_pred_*.txt` と
   `gen_win_backtest_data.sh` / `refresh_ev.sh` の生成分が今もこの形。後方互換を落とさないための行。
   **この 2 本の shell は golden に拘束されない**（`echo` で自前生成しており、勝手に変えても
   ここのテストは落ちない）。触るときは解析側の regex と手で突き合わせること

行を足すときは、Rust 側（1〜3 の生成物と一致）と Python 側（全行がパースできる）の
両方のテストを同時に更新すること。

見出しの**書式そのもの**（末尾スペースの有無を含む）を変えるときは、Rust の `race_heading` /
このファイル / `scripts/predict-check/pred_header.py` の 3 点を同じ PR で触ること。
現在は未発走が `）---`、発走済が `）[発走済] ---` と `---` の前のスペースが非対称だが、
これは全角括弧の直後にスペースを置かない日本語表記を優先した結果で、regex は両方を受ける。
