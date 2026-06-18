# ADR 0022: REST API（read 基盤）サーバの追加 (Issue #33)

## ステータス
承認済み

## コンテキスト

Web GUI（#34）から予想・分析を利用できるようにする前段として、HTTP 経由で read 系機能を提供する REST API が必要になった。確率推定・レース一覧・出馬表・分析統計のロジックは既に `use-case`（`interactor/race`・`interactor/{horse,course,jockey,trainer}`）と Repository 実装（`rdb-gateway`）に存在し、現状は CLI バイナリ（predict / analyze 等）からのみ呼べる。

加えて、API である以上は仕様（OpenAPI）を一級の成果物として整備し、フロント（#34）との契約を明確にしたい。手書き仕様はコードと乖離しやすい。

本 Issue のスコープは read 基盤に限定し、状態変更を伴う write 系（セッション作成・outcome 記録）は別 Issue（#53）に分離する（1 PR = 1 トピック維持）。

## 決定

- クリーンアーキテクチャ規約（`~/.claude/rules/rust/architecture.md`）に従い、新規 crate `interface/rest-controller` と新規 app `apps/api-server` を追加する。
- read 系の use-case / Repository は基本的に**既存実装を再利用**する（`races_by_date` / `predict_race` / `*_stats`）。ただし出馬表単体取得の use-case メソッドは現状存在しない（`find_race_card` は Repository 側のみ）ため、依存方向を崩さないよう **use-case に `race_card(race_id)` を 1 つだけ新規追加**する。それ以外の新規追加は interface / apps 層に閉じる。
- read エンドポイント: `GET /api/races`・`GET /api/races/{race_id}`・`GET /api/races/{race_id}/prediction`・`GET /api/analyze/{kind}`。
- DB は当面 SQLite を継続（`PADDOCK_DB_URL` で切替可能）。
- **OpenAPI は utoipa による*コードファースト***で生成する。Swagger UI（`/docs`）と `openapi.json`（`/api-docs/openapi.json`）を配信し、`docs/api/openapi.json` をコミットして CI でコード生成結果との一致をスナップショット検証する。
- 認証ミドルウェアの差し込み口（no-op）を Apps 層に 1 箇所だけ用意する（認証本体は別 Issue）。

## 理由

- **既存ロジック再利用**: 確率推定（`predict_race`）・レース一覧（`races_by_date`）・分析（`*_stats`）は use-case に集約済みで、interface/apps を足すだけで API 化できる。出馬表取得だけは use-case メソッドが無いため `race_card` を 1 つ追加するが、handler から Repository を直叩きしてレイヤー責務を崩すことは避ける。
- **OpenAPI コードファースト（utoipa 採用）**: handler/schema の型注釈から生成するためコードと仕様が乖離しない。spec-first の手書き YAML は二重管理でズレやすく却下。utoipa は自己完結ライブラリで外部 API に依存せず、本プロジェクトの「自己完結する解を優先」方針に合致する。生成物を `docs/api/openapi.json` にコミットしスナップショット検証することで、レビューで仕様差分を可視化し更新漏れを防ぐ。
- **read / write 分離**: write 系（残高更新・トランザクション）は関心事が異なり、テスト・レビューの粒度を保つため #53 に分離する。
- **SQLite 継続**: 現データソースのまま GUI 化を進めるのが最小ステップ。PostgreSQL 移行は `PADDOCK_DB_URL` 切替で後から可能。
- **認証フックの口だけ用意**: 現状シングルユーザーだが、将来のマルチユーザー化を非破壊で迎えるための最小の布石（web-spa.md の方針）。

## 影響

- 新規 crate / app が増え、ワークスペースのビルド・テスト対象が広がる。
- `Interactor<R, P, F>` の 3 ジェネリクスを read 用途でも引き回す必要がある（predict/analyze の DI を踏襲）。read 専用トレイトへの分離は将来課題。
- OpenAPI 生成物 `docs/api/openapi.json` のスナップショットテストにより、API スキーマ変更時はコミットの更新が必須になる（意図しない契約変更の検出にもなる）。
- 状態バッジ等、複数リソースを合成した表示は SPA（#34）側の責務になる（#33 は素の read のみ）。
