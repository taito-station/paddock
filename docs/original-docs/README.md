# original-docs — 読み取り専用の一次資料（生素材）

knowledge を蒸留する**元になる未整理の一次資料**を置く場所。**ここのファイルは書き換えない**
（HVE の original-docs と同じ思想。source は改変せず、蒸留は knowledge 側で行う）。
全体像は [docs/knowledge/README.md](../knowledge/README.md)。

## 何を置くか

- netkeiba / JRA の挙動メモ（エンコーディング・ページ生成タイミング・DOM 構造など外部仕様の観察）
- 外部から持ち込んだ資料・仕様の写し
- 生のバックテスト/実績ログのうち、まだ ADR / knowledge に蒸留していないもの
- 調査の生ノート（後で qa → knowledge に昇華する前段）

## 何を置かないか

- 確定した運用ルール・ドメイン知 → `docs/knowledge/` or `docs/specifications/`
- 決定とその根拠 → `docs/adr/`
- コード・設定（リポジトリ本体で管理）

## 運用

1. 生素材をここに置く（RO）。
2. Claude が読んで欠落/不整合を検出し、`docs/qa/` に質問票を起票。
3. 回答済み qa を knowledge に差分マージ。original-docs 自体は残す（トレーサビリティ）。

> 一次資料は mdq の索引対象（`mdq.toml`）に含まれるので、`scripts/mdq search` で横断検索できる。
