# REST API（read 基盤）: 設計仕様

[Issue #33](https://github.com/taito-station/paddock/issues/33) / 関連: [#34 Web SPA](https://github.com/taito-station/paddock/issues/34)・[#53 セッション write API](https://github.com/taito-station/paddock/issues/53)・[web-spa.md](web-spa.md)

> 確認日: 2026-06-18

## 概要

Web GUI（#34）から予想・分析を使うための前段として、既存のクリーンアーキテクチャ（`domain` / `use-case`）を再利用した **REST API サーバ**を追加する。本フェーズのスコープは **read 系エンドポイントの基盤**まで（予想セッションの状態変更を伴う write 系は #53 に切り出す）。

API なので **OpenAPI 仕様を一級の成果物として整備する**。utoipa による**コードファースト**で、handler/schema のコードから OpenAPI を生成し、コードと仕様が乖離しない状態を保つ。

![REST API (read) レイヤー構成](diagrams/rest-api-read-architecture.svg)

> 図は手書き SVG（macOS で drawio エクスポートが不可のため、`.svg` を正本として手で保守する）。

## スコープ

### 本 Issue（#33）でやること

- 新規 crate: `src/interface/rest-controller`（actix-web の handler / router / schema / error）
- 新規 app: `src/apps/api-server`（常駐バイナリ。DI 構築・route 設定・OpenAPI マウント）
- read 系エンドポイント（後述）
- OpenAPI 仕様（utoipa コードファースト）＋ Swagger UI 配信＋リポジトリへ `openapi.json` をコミットし CI で同期チェック
- 認証ミドルウェアの差し込み口（no-op）を Apps 層に 1 箇所
- 統合テスト（temp SQLite を seed して各エンドポイントを叩く）

### やらないこと（別 Issue）

- 買い目推奨 `GET /races/{id}/recommendations`（保存オッズ #51 が前提 → #51 完了後）
- セッション write 系（作成 / outcome 記録）→ #53
- オッズ・確定結果の refresh（ライブ取得→保存）→ #51 / #40
- 認証本体（JWT/argon2）→ マルチユーザー化の専用 Issue
- フロントエンド（SPA）→ #34
- PostgreSQL 移行（当面 SQLite を継続。`PADDOCK_DB_URL` で切替可能なまま）

## レイヤー構成と依存方向

`~/.claude/rules/rust/architecture.md`（クリーンアーキテクチャ規約）に従い、依存方向 **Apps → Interface → Use-Case → Domain** を厳守する。read 系の use-case（`interactor/race`・`interactor/{horse,course,jockey,trainer}`）と Repository 実装（`rdb-gateway`）は**既存のものをそのまま再利用**し、新規追加は interface（rest-controller）と apps（api-server）に閉じる。

| レイヤー | crate | 本 Issue での扱い |
|---|---|---|
| Apps | `apps/api-server` | 新規。常駐バイナリ・DI・route・OpenAPI マウント・認証フック |
| Interface | `interface/rest-controller` | 新規。handler / router / schema / error |
| Interface | `interface/rdb-gateway` | 既存。read メソッドのみ使用 |
| Use-Case | `use-case` | 既存。read interactor を再利用 |
| Domain | `domain` | 既存。schema で DTO 化して公開 |

### Interactor のジェネリクス（実装上の注意）

現行の `Interactor` は `Interactor<R: Repository, P: PdfParser, F: PdfFetcher>` の 3 ジェネリクスを持つ。read エンドポイントは `R`（Repository）しか使わないが、型としては `P` / `F` も必要になる。**既存の `apps/predict`・`apps/analyze` の `setup.rs` が同じ `Interactor<R,P,F>` を構築済み**なので、api-server の DI もそれを踏襲して同じ具象型を組み立てる（read 経路では P/F は呼ばれない）。

> P/F を read 用途で型から外す（read 専用トレイトへ分離する）リファクタは有効だが影響範囲が広いため本 Issue では行わず、必要になった時点で別 Issue とする。

## エンドポイント仕様

全エンドポイントは prefix `/api` の下に置く。`race_id` はドメインの `RaceId` 値オブジェクトの文字列表現をパスに使う。

### 1. レース一覧

```
GET /api/races?date=YYYY-MM-DD
```

- use-case: `find_races_by_date(date)`（race_num 昇順、`results` は読まない）
- `date` 必須・`YYYY-MM-DD`。不正フォーマットは `400`。
- レスポンス: レース配列

```json
{
  "date": "2026-03-28",
  "races": [
    { "race_id": "...", "venue": "nakayama", "race_num": 1, "distance": 1800, "surface": "turf" }
  ]
}
```

> 状態バッジ（未処理 / 購入済み / オッズ未取得 等）はセッション(#53)・オッズ(#51) の情報を要するため #33 では返さない。SPA 側が複数 read を合成して表示する（web-spa.md 参照）。

### 2. 出馬表（race card）

```
GET /api/races/{race_id}
```

- use-case: `find_race_card(race_id)`。`None` は `404`。
- レスポンス: レース諸元 + 出走馬（`HorseEntry`）

```json
{
  "race_id": "...",
  "date": "2026-03-28",
  "venue": "nakayama",
  "distance": 1800,
  "surface": "turf",
  "entries": [
    { "gate_num": 1, "horse_num": 1, "horse_name": "…", "jockey": "…", "trainer": "…", "weight_carried": 55.0 }
  ]
}
```

`jockey` / `trainer` / `weight_carried` は出典により欠落しうる（PDF 出馬表は騎手・調教師・斤量が無い）ため `null` 許容。

### 3. 確率推定

```
GET /api/races/{race_id}/prediction[?track_condition=&blend_alpha=]
```

- use-case: `predict_race(race_id, blend_alpha, track_condition)`
- 既定は **モデルのみ**（`blend_alpha=None`）・馬場未指定（`track_condition=None`）。本番 predict と同じ `EstimationConfig::production()` 経路。
- `track_condition`（任意）: `good|good_to_firm|...`（`TrackCondition` の文字列表現）。不正値は `400`。
- `blend_alpha`（任意）: `0.0..=1.0` の f64。市場オッズ（単勝）とのブレンド係数（#72）。範囲外・非有限は `400`。
- 出馬表が無い `race_id` は内部で `NotFound` → `404`。
- レスポンス: 馬ごとの win/place/show 確率（`win ≤ place ≤ show` 単調性は use-case が保証）

```json
{
  "race_id": "...",
  "probabilities": [
    { "horse_num": 1, "horse_name": "…", "win_prob": 0.18, "place_prob": 0.34, "show_prob": 0.49 }
  ]
}
```

### 4. 分析統計

```
GET /api/analyze/horse?name=<馬名>
GET /api/analyze/jockey?name=<騎手名>
GET /api/analyze/trainer?name=<調教師名>
GET /api/analyze/course?venue=<場>&distance=<m>&surface=<turf|dirt>
```

- use-case: `horse_stats` / `jockey_stats` / `trainer_stats` / `course_stats`（いずれも `as_of=None`＝全期間集計）
- 名前系は `name` 必須（`TryFrom` のドメインバリデーション、不正は `400`）。`course` は `venue`/`distance`/`surface` 必須。
- レスポンス: `*StatsRow` を JSON 化（`overall` と各カテゴリ別 `GroupStat`：`label / starts / wins / places / shows` ＋算出レート `win_rate / place_rate / show_rate`）

```json
{
  "horse_name": "…",
  "overall": { "label": "overall", "starts": 12, "wins": 3, "places": 5, "shows": 7,
               "win_rate": 0.25, "place_rate": 0.417, "show_rate": 0.583 },
  "by_surface": [ /* GroupStat[] */ ],
  "by_distance_band": [ /* … */ ],
  "by_gate_group": [ /* … */ ],
  "by_track_condition": [ /* … */ ],
  "by_popularity_band": [ /* … */ ]
}
```

> 名前あいまい検索（部分一致・カタカナ正規化, #50）は本 Issue では扱わない。完全一致でドメイン値に変換できた名前のみ受ける。

## OpenAPI（utoipa コードファースト）

API の仕様乖離を防ぐため、OpenAPI はコードから生成する（spec-first の手書き YAML は採用しない）。

- **依存**: `utoipa`（derive で `ToSchema` / `IntoParams` / `#[utoipa::path]`）、`utoipa-swagger-ui`（Swagger UI 配信）。いずれも自己完結ライブラリで外部 API には依存しない。
- **スキーマ注釈**: `schema/` の request/response 型に `#[derive(ToSchema)]`、handler に `#[utoipa::path(...)]` を付け、`#[derive(OpenApi)]` の `ApiDoc` に paths/components を集約する。
- **配信**: api-server が
  - `GET /api-docs/openapi.json` … OpenAPI ドキュメント（JSON）
  - `GET /docs` … Swagger UI
  をマウントする。
- **リポジトリへのコミットと同期チェック**: `ApiDoc::openapi()` をシリアライズした `docs/api/openapi.json` をコミットする。`api-server` の統合テスト（または `cargo test`）に「生成結果が `docs/api/openapi.json` と一致する」スナップショットテストを置き、差分があれば失敗させる（仕様の更新漏れを CI で検出）。生成し直しは同テストの更新手順に従う。
- **認証**: no-op 段階のため security scheme は定義しない（マルチユーザー化 Issue で `bearerAuth` 等を追加）。

## エラーマッピング

`rest-controller` の `error/mod.rs` に HTTP 用 `Error` enum と `ResponseError` 実装を置き、`use_case::Error` から `From` で変換する（規約のマッピングどおり）。

| use_case::Error | HTTP | 例 |
|---|---|---|
| `InvalidArgument` | 400 | 不正な日付・クエリ・ドメイン値変換失敗 |
| `NotFound` | 404 | 出馬表が無い race_id |
| `ExternalServer` | 500 | DB エラー等 |

エラーレスポンスは JSON で返す（例 `{ "error": { "code": "not_found", "message": "race card: ..." } }`）。

## Apps 層（api-server）

- `config.rs`: `PADDOCK_DB_URL`（既定 SQLite）・`SERVER_*`（bind アドレス/ポート）・`LOG_*` を環境変数から読む（既存 app の流儀に合わせる）。
- `setup.rs`: ロガー初期化 → SQLite プール → `RdbGateway`（Repository 実装）→ `Interactor<R,P,F>` 構築（predict/analyze と同じ具象 P/F）。
- `app.rs`: `configure_routes<R,P,F>` で rest-controller の各 router を `/api` 配下にマウント。**認証ミドルウェアの差し込み口を 1 箇所**用意（現状 no-op：素通し。将来ここに JWT 検証を挿す）。OpenAPI（`/docs`・`/api-docs/openapi.json`）もここでマウント。
- `bin.rs`: エントリポイント（`HttpServer` 起動）。

## マルチユーザー化への布石（今は実装しない）

- セッション系のパスは将来 `user_id` スコープを差し込めるリソース指向にする（#53 で `/sessions/{date}` を設計）。本 Issue の read パスも `/api/...` 配下で破壊せず拡張できる形にする。
- 認証ミドルウェアの差し込み口（no-op）を Apps 層に 1 箇所だけ用意する（上記）。

## テスト方針

- 統合テスト `src/apps/api-server/tests/`（規約どおり `helper/mod.rs` に temp SQLite 構築・seed・`App` 構築を集約）。
  - 各 read エンドポイントの正常系（200 + JSON 形状）、`404`（未存在 race_id）、`400`（不正クエリ）。
  - OpenAPI: `GET /api-docs/openapi.json` が 200 で返り、コミット済み `docs/api/openapi.json` と一致すること。
- 既存の CLI 群（parse-pdf / predict / analyze 等）はバッチ用途として変更しない。

## 関連 Issue / 参考

- #33 本 Issue（read 基盤）
- #53 セッション write API / #34 SPA / #35 docker-compose
- #51 単複オッズ永続化（recommendations の前提）/ #40 確定結果自動取得 / #50 名前あいまい検索
- `~/.claude/rules/rust/architecture.md`・`conventions.md`（クリーンアーキテクチャ／コーディング規約）
- ADR: `docs/adr/0022-rest-api-read-server.md`
