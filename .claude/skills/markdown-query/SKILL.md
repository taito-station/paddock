---
name: markdown-query
description: >
  ローカルの Markdown ドキュメント（docs/ 配下: adr / specifications / knowledge / qa /
  original-docs）から、ファイル全体を読まずに関連チャンクだけを取り出して答える。完全ローカル
  （外部 API なし・BM25 語彙検索）。USE FOR: プロジェクト文書からの回答、仕様・ADR・knowledge の
  検索、要件やバックテスト履歴の探索、対象ファイルパスが未知の横断検索。PREFER OVER: 対象が
  Markdown で複数ファイル横断・関連度順ヒットが欲しいときは Read/cat/grep より先にこれを試す。
  DO NOT USE FOR: Markdown の編集・生成、ソースコード(.rs/.ts)検索、埋め込み/クラウド検索。
  WHEN: docs 内に答えがありそうだがファイルやパスが未確定なとき／Context を節約したいとき。
metadata:
  origin: user
  version: 0.5.0
category: research
---

# markdown-query（mdq）

`docs/` 配下の Markdown を BM25 で横断検索し、ヒットした**小さな snippet だけ**を返す。生ファイルを
読み込む前にこれを使い、Context 消費を抑える（HVE 実測で全 .md 直読み比 ~99.8% トークン削減）。
実体は `tools/mdq/`（HVE 由来・MIT）、索引対象は `mdq.toml`。索引は `.mdq/*.sqlite`（gitignore・
セッション毎に再ビルド前提）。

## 最短手順（コピペ可）

```sh
scripts/mdq stats                                              # 索引の有無を確認
scripts/mdq index                                             # 未作成/古ければ実行（増分・自動prune）
scripts/mdq search --q "<質問の主要キーワード>" --top-k 5 --max-tokens 800
# 既定 --strategy auto（クエリから chunking を自動選択）。手動なら --strategy heading 等。
scripts/mdq get --chunk-id <返ってきた ID>                    # snippet で足りないときだけ本文取得
```

初回のみ venv セットアップ:

```sh
python3 -m venv tools/mdq/.venv
tools/mdq/.venv/bin/pip install -r tools/mdq/requirements.txt
```

## 使い方の要点

1. **索引**: `scripts/mdq index`。`mdq.toml` の `[index].roots`（docs/adr, docs/specifications,
   docs/knowledge, docs/qa, docs/original-docs）を走査。存在しない dir は自動スキップ。増分更新。
2. **検索**: `scripts/mdq search --q "クエリ" --top-k 5 --max-tokens 800`。出力は JSONL（1 行 1 ヒット、
   `path` / `heading_path` / `lines` / `score` / `snippet`）。`--paths "docs/adr/*"` で絞ると精度向上。
   `--mode grep` で完全一致に切替。
3. **本文取得**: `scripts/mdq get --chunk-id <ID>`（必要時のみ）。
4. 結果は**そのまま使う**（生 Markdown を読み直さない）。

## フォールバック
- ヒット 0 件 → キーワードを変えて 1〜2 回再試行 → `scripts/mdq list` で見出し俯瞰 → それでも不明なら
  serena（`mcp__serena__*`）やファイル読込へ。
- 索引対象は `.md` のみ。コード検索は serena、非 Markdown は通常ツールを使う。

## 補足
- semantic_paragraph / watch は追加依存（fastembed/nltk/numpy/watchdog）が必要で既定未導入。BM25 で運用する。
- 索引 DB(.mdq/) はコミットしない。ブランチ切替・環境更新後は `scripts/mdq index` で作り直す。
