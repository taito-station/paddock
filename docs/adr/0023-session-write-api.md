# ADR 0023: 予想セッション write 系 REST API (Issue #53)

## ステータス
承認済み

## コンテキスト

read 基盤（#33, ADR rest-api-read-server）に続き、Web SPA（#34）が予想セッションを GUI 上で完結できるよう、状態変更を伴う write 系 API が必要になった。CLI `predict`（`apps/predict/src/session.rs`）には残高ガード・1 開催日 1 セッション・per-bet 払戻・1 トランザクション保存・P&L 恒等式といった不変条件があるが、これらは **CLI app 層にのみ実装**されており、use-case の session 系メソッドは薄い委譲でガードを持たない。API がそれらを直接呼ぶと不変条件が二重実装・乖離する。

また read API では `use_case::Error` に二重作成（409 相当）を表す variant が無い。

## 決定

- `rest-controller` にセッション write 系エンドポイントを追加する:
  - `POST /api/sessions/{date}`（作成・二重作成ガード）
  - `GET /api/sessions/{date}`（収支サマリ + 明細）
  - `POST /api/sessions/{date}/races/{race_id}/outcome`（賭け金・払戻記録、残高ガード、1 トランザクション）
  - `POST /api/sessions/{date}/races/{race_id}/odds:refresh`（#51）
  - `POST /api/sessions/{date}/results:refresh`（#40）
- **不変条件を use-case 層の新メソッド**（`create_predict_session` / `record_race_outcome` / `session_summary`）に集約し、API と CLI で共有する。CLI も同メソッド経由に寄せ、不変条件の二重実装を作らない。
- `use_case::Error` に **`Conflict(String)`** を追加し、rest-controller で 409 に写像する。残高超過・budget 不正は 400、外部取得失敗（odds/results refresh）は `Fetch`/`Timeout` を 502、それ以外は 500。
- OpenAPI は #33 と同じ utoipa コードファーストで拡張し、`docs/api/openapi.json` をスナップショット検証する。
- パスは `/api/sessions/{date}` のリソース指向とし、将来 `user_id` スコープを非破壊で差し込める形にする（認証本体・DDL の `user_id` 化は別 Issue）。

## 理由

- **不変条件の単一実装**: 残高・トランザクション・状態遷移は金額に直結する正しさの要。CLI と API で重複させると乖離リスクが高いため use-case に集約する（クリーンアーキテクチャの責務配置にも合致）。
- **`Conflict` の追加**: 二重作成は REST 的に 409 が自然。既存 variant（400/404/500）への押し込みは意味を歪めるため、最小限の enum 拡張で正しいセマンティクスを与える。
- **read 基盤の再利用**: #33 の rest-controller / api-server / エラー写像・OpenAPI 流儀をそのまま拡張でき、追加コストが小さい。
- **#51 / #40 の再利用**: odds:refresh は `OddsInteractor`（ADR 0005）、results:refresh は `SettleInteractor::settle_session`（#40）を呼ぶだけで、取得ロジックを再実装しない。

## 影響

- `use_case::Error` に variant 追加。read 系（#33）の rest-controller の `From<use_case::Error>` は網羅性のため `Conflict` 分岐の追加が必要（コンパイラが検出）。
- use-case に不変条件付きの session メソッドが増え、CLI を同経路へ寄せるリファクタが発生する（範囲を抑えるため対話 UX は CLI に残す）。
- `predict_sessions` の一意制約は当面 `date` のまま。将来のマルチユーザー化で `(user_id, date)` へ拡張する前提を崩さない。
- write API のため回帰検知には Postgres 実行の統合テストが要る（#160 の CI 整備に依存）。
