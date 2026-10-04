# ツール利用ポリシー

HVE プロジェクトでのコード/ドキュメント探索ツールの利用優先順位。

## 探索ツールの優先順位

1. **cq (Code Query)**: ソースコード検索に最優先で使用
   - `python3 -m cq search --q "検索クエリ" --profile <profile>`
   - SQLite + BM25 + tree-sitter ベースのローカル検索エンジン
2. **mdq (Markdown Query)**: Markdown/CSV ドキュメント検索に使用
   - `python3 -m mdq search --q "検索クエリ" --top-k 5`
   - SQLite + BM25 ベースのローカル検索エンジン
3. **LSP ツール**: シンボル検索・参照検索に使用（その言語の LSP plugin が有効な場合）
   - Claude Code 組み込みの `LSP` ツール（`goToDefinition`, `findReferences`, `workspaceSymbol`）
   - deferred tool なら ToolSearch で読み込んでから使う。`workspaceSymbol` も対象言語の任意のファイルを `filePath` に渡す
4. **grep / find**: cq/mdq/LSP が利用できない場合のフォールバック。`grep` で対象を絞ってから `Read` する（無計画な全文検索・全読みはしない）

paddock では同じ検索を `scripts/cq search --q "..."` / `scripts/mdq search --q "..."` で行う（ルート `CLAUDE.md` の探索規律が正・paddock 固有）。

## 利用判断

- cq/mdq がインストール済みかは `python3 -m cq --help` / `python3 -m mdq --help` で確認
- 未インストール時は grep/find にフォールバックし、セッション中にインストールを推奨しない（中断を避ける）
- LSP ツールはシンボルの定義・参照検索に特化。テキスト全文検索には cq/mdq を使う

## cq/mdq のインストール

`setup.sh` が自動でインストールする。手順:

1. ローカルに HypervelocityEngineering リポがあれば環境変数で指定:
   ```
   HVE_REPO_PATH=/path/to/HypervelocityEngineering bash setup.sh /path/to/target
   ```
2. 指定がなければ hve-playbook の兄弟ディレクトリ (`../HypervelocityEngineering`) を探す
3. 見つからなければ自動で `git clone` してインストール

手動インストール:
```
python3 -m pip install -e 'path/to/HypervelocityEngineering[code,mdq]'
```
