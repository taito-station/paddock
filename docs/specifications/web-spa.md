---
# knowledge 規約に基づくメタデータ（docs/knowledge/README.md）。specifications はその場で
# knowledge に昇格（ADR 履歴・相互リンクを壊さないため物理移動しない）。
status: Confirmed
kind: knowledge
sources:
  - docs/adr/0068-race-result-ingestion-ui-reflection.md
  - docs/adr/0019-portfolio-generator.md
  - docs/adr/0046-allocation-prob-weight-no-floor-rejected.md
  - docs/adr/0054-kelly-staking-rejected.md
distilled_from_sha: "f765be7"
updated: "2026-07-17"
---

# Web フロントエンド (SPA): 機能仕様

[Issue #34](https://github.com/taito-station/paddock/issues/34) / 依存: [#33 REST API（read 基盤）](https://github.com/taito-station/paddock/issues/33) ・ [#53 セッション write API](https://github.com/taito-station/paddock/issues/53)

## 概要

REST API (#33) を消費する **Web SPA** を追加し、CLI `predict` の対話セッションを GUI に置き換える。
閲覧（確率推定・買い目推奨・分析・過去収支）に加え、**予想セッション（賭け金入力・払戻記録・残高管理）を Web 上で完結**させる。CLI `predict` は当面バッチ／オフライン用途として併存する。

本ドキュメントは**機能スコープと画面・データフローの確定**を目的とし、フレームワーク選定・ビジュアルデザイン（配色・コンポーネント設計）は別途とする。

## 前提となる決定事項

| 項目 | 決定 | 設計上の含意 |
|------|------|------------|
| 役割範囲 | 閲覧 + 予想セッション完結（CLI predict の GUI 置換） | 状態を持つセッション操作を API/SPA 双方で扱う |
| 利用者 | **現状シングルユーザー（認証なし）**。最終的にマルチユーザー化 | 認証は今は作らない。ただし session/bets スキーマと API パスは後から `user_id` を非破壊で足せる形にする |
| データ鮮度 | 永続化済み（#51 オッズ / #40 確定結果）を表示 + 「最新取得」手動更新ボタン | 既定は自動ポーリングしない。**例外（#381・ADR 0068）: 当日・発走済み・未確定のレースが残る間だけ結果取り込み（`results:refresh`）を自動ポーリングし、全確定で停止**。過去日・確定済みは従来どおり自動更新しない |

---

## 画面構成（ビュー）

```
[1] レース一覧 (日付選択)
      └─[2] レース詳細 / セッション操作
              ├─ 確率表 (win/place/show)
              ├─ 買い目推奨 (EV・均等配分)
              └─ 賭け金入力 → 払戻記録
      └─[3] セッション収支 (1開催日サマリ)
[4] 分析 (horse / course / jockey)
```

### [1] レース一覧ビュー

- 日付を選び、その開催日のレース一覧を表示（venue / レース番号 / 距離 / 芝ダ / 発走時刻）。
- 各レースに状態バッジを出す: `未処理` / `購入済み` / `スキップ` / `オッズ未取得` / `出馬表なし`。
- **「終了」（⚫終）は結果確定（`result_confirmed`）で判定する（#381）。** `post_time` 経過だが未確定の
  レース（走行中/結果待ち）は「終了」にせず未発走側に残す。着順が取り込まれて確定した時点で ⚫終・着順・
  的中/払戻（購入レースは session `bets[].payout`）を表示する。
- 発走時刻の表示・自動ポーリングの gate には `/api/races` の `post_time`（race_cards 正本、#391）を使う
  （ライブ EV の `post_time` は snapshot 時点の複写で fallback）。post_time 不明（`null`）は未発走側（発走済みと断定しない）。
- 開催日単位のセッション状態（未作成 / 進行中 / 完了）と残高を表示。新規開始時は **budget 入力**を要求する（CLI の `--budget` 相当）。

### [2] レース詳細 / セッション操作ビュー

CLI `run_race` の対話ループを画面化する。

1. **確率表**: 馬番・馬名・勝率・連対率・複勝率（`predict_race` の出力）。
2. **買い目推奨**: 各買い目を券種・組合せ・EV・推奨額で表示。本番配分は `build_portfolio`（ワイド・馬連・三連複の◎軸ながし、券種予算を 100 円単位で均等配分。[ADR 0019](../adr/0019-portfolio-generator.md)。券種内の均等配分は [ADR 0046](../adr/0046-allocation-prob-weight-no-floor-rejected.md) で確率重み化を棄却し維持）。Kelly 配分は [ADR 0054](../adr/0054-kelly-staking-rejected.md) で棄却済みで、`select_bets`/Kelly は backtest 評価専用。
   - 閾値超えの買い目が無い場合は「該当なし」を明示。
   - オッズ未取得（#51 未保存）の場合は推奨を出さず、**「最新取得」ボタン**でライブ取得 → 保存 → 再計算を促す。
3. **購入方法**: CLI の `y / e / s` に対応する 3 操作。
   - `推奨通り`: 推奨額をそのまま採用。
   - `編集`: 買い目ごとに賭け金を入力（合計が残高超過なら確定不可・バリデーション表示）。
   - `スキップ`: 賭けなしで次へ。
4. **払戻記録**: レース確定後、賭けた買い目ごとに払戻額を入力（#40 で確定結果が保存済みなら自動補完 → 手動上書き可）。
5. 確定すると残高・総賭け金・総払戻を更新し、**セッション + 当該レースの買い目を 1 トランザクションで保存**（CLI `save_race_outcome` 相当の API）。

### [3] セッション収支ビュー

- CLI `--summary` 相当。開始予算・現在残高・総賭け金・総払戻・P&L・回収率を表示。
- 買い目明細テーブル（レース / 券種 / 組合せ / 賭け金 / 払戻 / EV）。
- 未完了セッションは「進行中」と明示し、[2] への再開導線を出す（CLI `--resume` 相当）。

### [4] 分析ビュー

- `horse` / `course` / `jockey` 統計を表示（#33 の read エンドポイント）。
- 馬名・騎手名・調教師名検索は**部分一致・カタカナ正規化**（#50 の normalizer を `/candidates` で REST 露出・#401）。
  入力→候補（`/analyze/{kind}/candidates?q=`）→ 1 件は自動確定・多数は一覧クリックで確定 → 統計（`/analyze/{kind}?name=`）。

---

## データフロー / 鮮度

```
[永続化 DB] ──read──▶ API ──read──▶ SPA（既定表示）

[永続化 DB] ◀──write── API ◀──action── SPA「最新取得」ボタン
   (odds: #51 ライブ取得→保存 / 結果: #40 取得→保存)
```

- 既定はすべて**永続化済みデータ**を表示する（再現性重視・自動更新なし）。
- オッズ未保存・結果未確定のレースでのみ「最新取得」ボタンを出し、押下時に API がライブ取得 → 保存 → 最新値を返す。
- **例外（#381・ADR 0068）: 結果の自動反映**。ライブ一覧・収支サマリは、当日・発走済み・未確定のレースが
  残る間だけ `POST /api/results/{date}:refresh`（冪等・`force=false`）を自動ポーリングし、着順取り込み＋自動精算を
  進める。新規確定で races/live/session を無効化して着順・的中/払戻・残高を反映し、全確定で停止する。
  収支サマリの手動「精算」ボタン（`force=true` 委譲エイリアス）はフォールバックとして残す。過去日は自動更新しない。
- セッションの賭け金・払戻記録は常に DB を正とし、楽観的 UI 更新後にサーバ確定値で整合させる。

---

## マルチユーザー化への布石（今は実装しない）

最終的なマルチユーザー化に備え、シングルユーザー実装の段階で以下だけ守る:

- `predict_sessions` / `predict_bets` のキー設計を「後から `user_id` 列を追加しても一意性が壊れない」形にしておく（現状の `1開催日=1セッション` 一意制約を将来 `(user_id, date)` に拡張できるよう DDL を整理）。
- API パスは将来 `user_id` スコープを差し込めるよう、セッション系を `/sessions/{date}` のリソース指向で設計する。
- 認証ミドルウェアの差し込み口（現状は no-op）を Apps 層に1箇所用意する。

> 認証本体（JWT/argon2）はマルチユーザー化の専用 Issue で、プロジェクトの Rust アーキテクチャ規約（クリーンアーキテクチャの認証パターン）に従って実装する。本フェーズでは作らない。

---

## API エンドポイント（#33 で実装、本 SPA が消費）

read 系（#33 スコープ）に加え、セッション操作の write 系が必要。**#33 の現スコープは read 系のみ**のため、セッション write 系は #53（予想セッション write API）で実装する。

| メソッド | パス | 用途 | 依存 |
|---------|------|------|------|
| GET | `/races?date=YYYY-MM-DD` | レース一覧 + 状態 | #33 |
| GET | `/races/{race_id}/prediction` | 確率推定 | #33 |
| GET | `/races/{race_id}/recommendations` | 買い目推奨（保存オッズ基準） | #33, #51 |
| POST | `/races/{race_id}/odds:refresh` | オッズをライブ取得して保存 | #51 |
| GET | `/analyze/{kind}` | 分析統計（kind: horse / course / jockey / trainer, `?name=` 完全一致） | #33 |
| GET | `/analyze/{kind}/candidates?q=` | 部分一致候補（kind: horse / jockey / trainer） | #50, #401 |
| GET | `/sessions/{date}` | セッション収支 + 明細（summary） | #53 |
| POST | `/sessions/{date}` | セッション新規作成（budget 指定） | #53 |
| POST | `/sessions/{date}/races/{race_id}/outcome` | 賭け金・払戻を記録（残高更新, 1 トランザクション） | #53 |
| POST | `/sessions/{date}/results:refresh` | 確定結果を取得して払戻自動補完 | #40 |

---

## スコープ外（本フェーズでやらない）

- 認証・マルチユーザーのデータ分離（布石のみ）。
- リアルタイム自動更新（WebSocket / 常時ポーリング）。**例外**: 当日・未確定レースに限る結果取り込みの
  自動ポーリングは #381（ADR 0068）で導入済み（全確定で停止・過去日は対象外）。恒常的な全画面ポーリングはやらない。
- ビジュアルデザイン・フレームワーク選定（別 Issue）。
- 馬券の実購入連携（IPAT 等）。あくまで記録・分析にとどめる。

---

## 関連 Issue

- #33 REST API（read 系の基盤）
- #53 セッション write API（作成 / outcome 記録 / odds・results 更新）
- #34 本 SPA
- #35 docker-compose（web サービスとして配信）
- #40 確定結果の自動取得（払戻自動補完）
- #51 単複オッズ永続化（推奨計算の基盤）
- #50 名前あいまい検索（分析ビュー）
