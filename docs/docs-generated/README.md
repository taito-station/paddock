# docs-generated — 自動生成ドキュメント

ソースコードや設定から自動生成される技術文書の置き場所。
HVE（dahatake/HypervelocityEngineering）の `docs-generated/` と同じ概念。

手書きのドキュメントはここに置かない。
- 確定知は `docs/knowledge/`
- 仕様は `docs/specifications/`
- 一次資料は `docs/docs-original/`
- 質問票は `docs/qa/`

## 想定される成果物

- `cargo doc` の出力（Rust API ドキュメント）
- OpenAPI スキーマ（`openapi.json` のスナップショット）
- その他ビルドプロセスが生成する文書

## 運用

- このディレクトリの中身は再生成可能なので、コミットは任意
- mdq の索引対象に含める場合は `mdq.toml` の `roots` に追加する
