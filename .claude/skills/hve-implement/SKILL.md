---
name: hve-implement
description: |
  implement-flow（Step 0〜8 のゲートフロー）を順序どおりに実行する実装ワークフロー。
  Plan → Knowledge 検索 → 棄却済み案チェック → ギャップ分析 → テスト設計 → 実装 → Knowledge 同期 → PR 作成。
  新機能・バグ修正・リファクタリング・既存機能の改修が対象。ドキュメントのみ・設定変更のみ・依存更新のみの変更には使わない。
  「/hve-implement」「implement-flow に沿って実装して」で起動。resolve-issue からも呼ばれる。
---

# hve-implement — implement-flow の実行

規約の正本は `.claude/rules/hve/implement-flow.md`。この skill は、その Step 0〜8 を実行する手順を定める。
規約と食い違ったら規約に従う。

## 着手前

- `.claude/rules/hve-local/hve-implement.md` があれば読み、この skill の手順と突き合わせてから進める
  （上書き・補足の扱いは `.claude/rules/hve/local-overrides.md`）
- `.claude/rules/hve-local/implement-flow.md` があれば併せて読む（Step 7 の検証コマンドなど）

## 呼び出しモード

| モード | 起動 | 実行するステップ |
|---|---|---|
| 単独 | `/hve-implement <実装したい内容>` | Step 0〜8 |
| resolve-issue の前段 | resolve-issue が `--phase pre <Issue 番号>` で呼ぶ | Step 2〜4 |
| resolve-issue の後段 | resolve-issue が `--phase post <Issue 番号>` で呼ぶ | Step 5〜7 |

- `--phase` が無ければ単独モード。`--phase` は resolve-issue からだけ渡す（単独で使うときは付けない）
- Issue を対応するときは resolve-issue を使う（パス判定・Plan の敵対的レビュー・PO 承認・Issue への記録は resolve-issue が持つ）。単独モードは Issue の無い実装指示の入口。resolve-issue skill が無ければ、implement-flow の対象（feat / fix / refactor / 改修）の Issue は単独モードで行い、Step 1 で `gh issue view <Issue 番号> --comments` を読む
- resolve-issue から呼ばれたときは、Step 0（ブランチ）・Step 1（Issue 分析）・Step 8（PR 作成）は resolve-issue が行う。
  Step 1 のスコープは `gh issue view <Issue 番号> --comments` で読み、前段の結果は resolve-issue の Plan に書く
- 前段は resolve-issue の Plan の作成前（bug パスは修正方針の記録前）に、後段は resolve-issue Step 3 の実装として呼ばれる。
  STOP が PO の承認より前に出るようにするため（承認済みの Plan を作り直さずに済む）

### 単独モードの承認

Step 0 のあと、Step 1〜4 を Plan モード（EnterPlanMode）で進める。Step 4 が終わったら、Step 2〜4 の結果
（検索で見つけた知識・棄却済み案との照合・質問票の回答と前提）と Step 1 の分類を計画に書き、ExitPlanMode で承認を得てから Step 5 へ進む。
Step 2〜4 の結果は見出し「ゲートの確認結果」の節に、関係する rules・skill ごとの準拠状況は見出し「ルール準拠表」の節に
`| ルール | 状態 | 対応 |` の Markdown の表で書く（見出しは `#`〜`####`）。
Plan モードで始めたとき（Step 0 の前に Plan モードに入っていたとき）は、Step 0 のブランチ作成を承認の後（Step 5 の前）に行う。
Plan モードを使えない環境（headless など）では、計画を提示して止まり、承認を得るまで Step 5 へ進まない。

## 手順

### Step 0: ブランチ作成

- デフォルトブランチから作業ブランチを切る。導入先にブランチ運用規約があればそれに従い、無ければ `<type>/<short-kebab-description>`
- ブランチ上でなければコードを書かない

### Step 1: Plan（スコープ特定）

- 指示の目的を理解する
- 影響する領域・モジュール・データモデルを列挙する
- Step 2 で検索するキーワード・領域を決める
- 単独モードでは、タスクを「バグ修正」か「それ以外」に分類する（Step 8 の `--depth` を決める。計画に書いて承認を得る。create-pr skill が無ければ分類しない）

| 結果 | 次のステップ |
|---|---|
| スコープ特定完了 | → Step 2 |

### Step 2: Knowledge 検索（Plan の各領域を網羅）

Step 1 で決めた領域ごとに、`.claude/rules/hve/tool-usage.md` の優先順位で検索する。

- ドキュメント: `python3 -m mdq search --q "<キーワード>" --top-k 5`（未導入なら `knowledge/`・`docs/` を grep で絞ってから Read）
- コード: `python3 -m cq search --q "<関数名・型名・概念>" --profile <profile>`（未導入なら grep で絞ってから Read）
- シンボルの定義・参照: 組み込みの `LSP` ツール（`workspaceSymbol` / `goToDefinition` / `findReferences`）

| 結果 | 次のステップ |
|---|---|
| 既存実装と矛盾 | STOP: 矛盾するコード箇所・知識を示して報告 |
| OK | → Step 3 |

### Step 3: 棄却済み案チェック

- Step 2 で見つけた決定ログの「却下した代替案」を読む。決定ログの置き場所は `.claude/rules/hve/artifact-management.md` の「決定ログの不変性」に従う
- これから取るアプローチが棄却済み案と衝突しないかを確かめる

| 結果 | 次のステップ |
|---|---|
| 棄却済み案と衝突 | STOP: 衝突する決定（ID・内容）と代替案を示して報告 |
| OK | → Step 4 |

### Step 4: ギャップ分析 → 質問票

Step 1〜3 の結果から、実装に必要だが足りない情報を「ブロッキング」と「軽微」に分ける。

- ブロッキング: 回答なしでは実装方針を決められない（仕様の解釈が複数ある、既存コードの意図が不明など）
- 軽微: 前提を明示すれば進められる（文言、ログレベルなど）

ブロッキングな不明点は質問票にして回答を得る。各設問に推奨解と理由を添える。

| モード | 質問の手段 | 回答の保存先 |
|---|---|---|
| 単独 | AskUserQuestion | 計画に書く |
| resolve-issue の前段（bug） | AskUserQuestion で PO に直接確認（resolve-issue の bug パスと同じ） | Issue コメント |
| resolve-issue の前段（feature） | resolve-issue Step 2 [feature] の「不確定点チェック」の探索順（knowledge → PO の質問票）に従う | `qa/`（resolve-issue の feature 前処理の定めどおり。PO の回答は questionnaire skill が書く `qa/*.md` を正とする。knowledge には AKM の蒸留で入れる）と Issue コメント |

| 結果 | 次のステップ |
|---|---|
| ブロッキングな不明点あり | 質問票提示 → 回答待ち |
| 回答がアプローチを変える | → Step 1（最大 2 周）。resolve-issue の前段では Step 2〜4 をスコープを直してやり直す |
| 回答で新たな不明点 | → Step 4 先頭（回答ごとに 1 回。連鎖したら STOP） |
| 軽微な不明点のみ | 前提を明示して → Step 5 |
| ギャップなし | → Step 5 |

単独モードはここで計画の承認を得る（「単独モードの承認」）。resolve-issue の前段はここで終わり、結果を resolve-issue に返す。

### Step 5: テスト設計

- 受入基準（何が動けば完了か）を箇条書きにする
- テストケースを設計する: 正常系、異常系（エラー・境界値）、既存機能の回帰
- 実装より先にテストを書き、失敗すること（RED）を実際のコマンド出力で確かめる

| 結果 | 次のステップ |
|---|---|
| 受入基準不明確 | → Step 4（最大 2 周）。resolve-issue の後段では STOP し、feature は Plan の修正と PO の再承認に、bug は修正方針の見直しに戻る |
| OK | → Step 6 |

### Step 6: 実装

- テストを通す最小限の実装を書き（GREEN）、必要なら整理する
- 全テストを実行して回帰が無いことを確かめる。画面を伴う実装はブラウザテストも行う
- resolve-issue の後段では、全テストを実行する時期とブラウザテストは resolve-issue（Step 3 のテスト・Step 4）に従う

| 結果 | 次のステップ |
|---|---|
| 同一テストが 3 回連続失敗 | STOP: アプローチの破綻を報告。resolve-issue の後段では、そのあと resolve-issue Step 3 の revert とエスカレーションに従う |
| 影響範囲が想定超 | STOP: 想定との差分を示して報告 |
| テスト通過 | → Step 7 |

### Step 7: Knowledge 同期

1. 実装で確定知の前提が変わったら、`knowledge/`・`docs/` の該当箇所を同じ PR で差分マージする（全面書き換えしない）
2. 設計判断を伴う変更は、決定ログに同じ PR で追記する。置き場所と書式は `.claude/rules/hve/artifact-management.md` に従う
3. 検証コマンドを実行する:
   - `.claude/rules/hve-local/implement-flow.md` に検証コマンドが列挙されていれば、それだけを実行する（引数を含めて hve-local の記述どおりにする）
   - 無ければ検査スクリプトを `.claude/scripts/hve/`（導入先）→ `hve-scripts/`（hve-playbook 自身）の順に探し、見つかった場所で実行する:
     ```bash
     python3 <場所>/check-knowledge.py
     python3 <場所>/check-decision-log.py
     ```
   - どちらにも無ければ、`.claude/rules/hve/knowledge-maturity.md` の「Stale 検出の仕組み」と artifact-management の「機械検査」の手順を手で行い、結果を報告（resolve-issue からは Issue コメント）に残す
   - 失敗したら直してから進む。直せなければ STOP

| 結果 | 次のステップ |
|---|---|
| Knowledge 更新で矛盾 | STOP: Conflict を宣言し、矛盾箇所を示して報告 |
| OK | → Step 8（resolve-issue の後段はここで終わり、resolve-issue に返す） |

### Step 8: PR 作成

1. 変更をコミットする（commit-workflow skill があればその規約に従う）
2. create-pr skill があれば Skill ツールで呼ぶ。`--depth` は承認された計画の分類（Step 1）で決める:
   - バグ修正 → `/create-pr --depth lightweight`
   - それ以外 → `/create-pr --depth full`
3. 2 の create-pr skill が無ければ、作業ブランチを push して `gh pr create` で PR を作る（本文は導入先のブランチ運用規約の雛形があればそれに従う）

| 結果 | 次のステップ |
|---|---|
| PR 作成完了 | 完了 |

## STOP 時の報告

STOP 条件に該当したら、黙って続行せずに次の形式で報告して止まる（implement-flow の「STOP 時の報告」）。

```
🛑 実装フロー停止

■ 停止ゲート: Step N（ゲート名）
■ 理由: <衝突内容・不明点・失敗内容など>
■ 試みたこと: <探索・分析・修正の内容>
■ 推奨アクション: <次にどうすべきか>
```

## 注意事項

- Step 1 を省略しない。スコープを決めずに検索すると、キーワードが足りず知識を取りこぼす
- Step 2 を省略しない。「知っているから」は理由にならない（棄却済み案の見落としを防ぐ）
- Step 5 を省略しない。テストが書けないなら受入基準が不明確
- Step 7 を後回しにしない。同じ PR で完結させる
