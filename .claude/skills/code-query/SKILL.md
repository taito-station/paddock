---
name: code-query
description: >
  ローカルのソースコード（src/ Rust, scripts/ Python/Bash, web/src/ TypeScript）を BM25 +
  symbol + regex で横断検索する。完全ローカル（外部 API なし）。USE FOR: ソースコードの横断検索、
  シンボル定義の探索、REQ-ID やコメントからコード実装の逆引き。PREFER OVER: grep/find でソースを
  探す前にこれを試す。DO NOT USE FOR: Markdown 検索（mdq を使う）、ファイル編集。
  WHEN: 実装箇所やシンボルの場所が未確定なとき／Context を節約したいとき。
metadata:
  origin: user
  version: 0.1.0
category: research
---

# code-query（cq）

`src/`, `scripts/`, `web/src/` 配下のソースコードを BM25 + symbol + regex で横断検索し、
ヒットした**小さな snippet だけ**を返す。生ファイルを読み込む前にこれを使い、Context 消費を抑える。
実体は `tools/cq/`（HVE 由来・MIT）、索引対象は `cq.toml`。索引は `.cq/*.sqlite`（gitignore・
セッション毎に再ビルド前提）。

## 最短手順（コピペ可）

```sh
scripts/cq stats                                              # 索引の有無を確認
scripts/cq index                                             # 未作成/古ければ実行（増分）
scripts/cq search --q "<質問の主要キーワード>" --top-k 5
scripts/cq def --symbol "function_name"                      # シンボル定義の場所
scripts/cq get --chunk-id <返ってきた ID>                    # snippet で足りないときだけ本文取得
```

cq は標準ライブラリのみで動作する（Python >= 3.11 必須。`tomllib` 依存）。
tree-sitter を導入するとシンボル抽出の精度が上がる（任意）:

```sh
python3 -m venv tools/cq/.venv
tools/cq/.venv/bin/pip install tree-sitter tree-sitter-rust tree-sitter-python
```

## 使い方の要点

1. **索引**: `scripts/cq index`。`cq.toml` の `[profiles.paddock].roots`（src, scripts, web/src）を
   走査。増分更新。`--rebuild` で全再構築。
2. **検索**: `scripts/cq search --q "クエリ" --top-k 5`。出力は JSONL。ルーティングは自動
   （trace ID → symbol → substring → regex → BM25 を試行し RRF で融合）。
   `--mode symbol` でシンボル検索に限定。`--paths "src/**"` でパス絞り込み。
3. **シンボル定義**: `scripts/cq def --symbol "predict"` でシンボル定義の場所だけを返す（本文なし）。
4. **参照**: `scripts/cq refs --symbol "build_portfolio"` でそのシンボルの参照箇所を列挙。
5. **トレース**: `scripts/cq trace --id "REQ-D23-002"` で REQ-ID がコードのどこに出現するか検索。
6. **リポマップ**: `scripts/cq map` でトークン予算付きのリポジトリ概要を出力。

## フォールバック
- ヒット 0 件 → キーワードを変えて再試行 → serena（`mcp__serena__*`）へ。
- 索引対象はソースコードのみ。Markdown 検索は mdq を使う。

## 補足
- 索引 DB(.cq/) はコミットしない。環境更新後は `scripts/cq index` で作り直す。
- 利用ログは `.cq/usage.jsonl`（gitignore・ローカルのみ）に記録される。
