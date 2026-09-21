# docs-generated — 自動生成ドキュメント

ソースコードや設定から自動生成される技術文書の置き場所。
HVE（dahatake/HypervelocityEngineering）の `docs-generated/` と同じ概念。

手書きのドキュメントはここに置かない。
- 確定知は `knowledge/`
- 仕様は `knowledge/`
- 一次資料は `docs-original/`
- 質問票は `qa/`

## 想定される成果物

- `cargo doc` の出力（Rust API ドキュメント）
- OpenAPI スキーマ（`openapi.json` のスナップショット）
- その他ビルドプロセスが生成する文書

## 運用

- このディレクトリの中身は再生成可能なので、コミットは任意
- 現時点では mdq の索引対象外。成果物が追加された時点で `mdq.toml` の `roots` に追加する
