# 0068. レース結果（着順・払戻）の同日取り込みと UI 自動反映（結果確定フラグ・自動精算）

## ステータス

提案中（設計書 PR レビュー中）。対象 Issue: [#381](https://github.com/taito-station/paddock/issues/381)。**本設計書 PR のマージ承認をもって「承認済み」に更新**する。本 ADR に伴う実装は承認後の別 PR（サーバ → web の順）。

## コンテキスト

開催当日のレース結果（着順・的中/不的中・払戻）が UI のどこにも出ず、結果照合が netkeiba 直接確認と収支画面の手動「精算」ボタンに依存している。ライブボードの「⚫終」判定も **発走時刻（`post_time`）の推定のみ**で、結果が確定したかを表す信号が API に無い（#370・#391 の実装時に確認）。

現状を調査したところ、**自動精算エンジンは #40 で既に実装済み**である。差分は「結果確定の信号化」と「UI への自動反映」に絞られる。

- `settle_bet()`（`domain/payout`）: 券種・組番・賭金・確定払戻から `Hit`/`Refund`/`Miss` を算出する純関数。返還（取消/除外馬）優先・16 ケースのテスト済み。
- `parse_race_payouts()`（`netkeiba-scraper`）: netkeiba 結果ページから確定払戻ブロック・取消馬・全額返還を抽出。
- `SettleInteractor::settle_session(date)`（use-case）: セッションの `predict_bets` を確定払戻と照合し payout・収支・回収率を更新。**毎回ゼロから再計算する冪等**設計。未確定レースは pending 据え置き、全レース確定で `completed=true`。
- `POST /api/sessions/{date}/results:refresh` / 収支画面の「精算」ボタン: 上記を手動起動。

一方、以下が**欠落**している。

- **結果確定フラグが無い**。ライブ一覧の「⚫終」は `post_time`（HH:MM）と現在時刻の相対推定に頼る（`web/src/lib/live.ts`）。実際に結果が出たか（着順確定か）を区別できない。
- **着順が同日は DB に無い**。`results` テーブルは PDF フロー（RO 一次資料）と `paddock-fetch-results`（既存行の **UPDATE 専用**・ADR 0015）でしか埋まらず、当日は出馬表（`race_cards`）しか無く着順行が存在しない。`settle_session` は払戻ブロックの有無だけを見て着順は保存しない。
- **精算が手動**。自動トリガーが無く、収支反映は人が「精算」ボタンを押す前提。

## 決定

**結果確定処理（着順・払戻の同日取り込み）を 1 つの冪等ユースケースに集約し、UI は既存の read クエリに結果フィールドを足して web ポーリングで自動反映する。**

### 1. 同日結果取り込み（サーバ）

新ユースケース `ResultsInteractor::refresh(date)` を追加する。対象は **開催日のレースのうち `post_time` を過ぎ、かつ未確定**のもの（発走前・確定済みは netkeiba を叩かずスキップ）。

1. 各対象レースにつき netkeiba 結果ページ（`race/result.html`）を **1 回だけ取得**し、同一 HTML から着順（`parse_race_result`）と払戻（`parse_race_payouts`）を **両方**パースする（結果ページに双方が載るため往復を二重化しない。ページング規律 ADR 0021/0029 準拠）。
2. 着順を `results` へ **upsert**（INSERT ... ON CONFLICT `(race_id, horse_num)` DO UPDATE、`source='netkeiba'`）。当日は着順行が無いため **INSERT 経路が新設点**（既存 `update_results` は UPDATE 専用のため別メソッド `upsert_results` を追加）。`results` の NOT NULL 列 `gate_num`/`horse_name` は結果ページに載らない/揺れる場合、既存 `race_cards`（出馬表）を `(race_id, horse_num)` で引いて補完する。
3. 同一パスでその払戻を使い、**セッションがあれば** `settle_bet` で精算する（払戻はこのパスで取得済みのものを in-memory で渡し、`settle_session` のような netkeiba 再取得をしない）。精算は従来どおり冪等・pending 据え置き・全額返還（#131）を踏襲する。
4. `SettleReport` に加え「新規確定したレース」を返す。

**結果確定フラグ `result_confirmed` は派生値**とし、専用カラムを増やさない（シンプル第一）。定義: 当該 `race_id` に着順（`finishing_position IS NOT NULL` を 1 行以上、または全馬取消/中止の確定）を持つ `results` 行が存在すること。これにより **賭けていない・スキップしたレースも**「終」を確定でき、着順表示もできる。

### 2. API への結果公開（read）

既存 read の DTO に結果フィールドを足す（新規 read エンドポイントは作らず、web の既存クエリに相乗り）。

- `GET /api/races`（`RaceSummary`）: `result_confirmed: bool` と `finish_order: [{position, horse_num, horse_name}]`（上位 3）を追加。ライブ一覧の「終」バッジ・着順表示の一次ソース。
- `GET /api/races/{race_id}/board`（`BoardHorseSchema`）: `finishing_position: Option<u32>` を各馬に、`result_confirmed: bool` を盤に追加。1 レース盤の結果反映。
- `GET /api/live/{date}`（`LiveRaceViewSchema`）: `result_confirmed: bool` を追加（「⚫終」を推定から確定へ置換）。的中/払戻は既存 `GET /api/sessions/{date}` の `bets[].payout` から web が算出する（別ソースを増やさない）。
- 書き込み口: `POST /api/results/{date}:refresh` を新設し `ResultsInteractor::refresh` を起動。既存 `POST /api/sessions/{date}/results:refresh` は本フローへ委譲する薄いエイリアスに変更（後方互換）。
- **OpenAPI を一級成果物**とする（utoipa コードファースト＋`openapi.json` スナップショット更新・検証を DoD 化）。

### 3. UI 自動反映（web）

- **自動精算トリガーは web ポーリング駆動**。ライブ一覧／収支サマリで、`post_time` を過ぎ未確定のレースが残る間だけ `POST /api/results/{date}:refresh` を 30–60 秒間隔で叩く。`ResultsInteractor` は冪等なので何度叩いても安全。**全レース確定でポーリング停止**（netkeiba への無駄打ちを止める）。手動「精算」ボタンは**フォールバックとして残す**。
- ライブ一覧の発走済み行に **的中○/✗・払戻額**（session `bets[].payout` 由来）と **着順**（`finish_order` 由来）を表示。「⚫終」は `result_confirmed` で判定（`post_time` 推定を置換）。`post_time` は発走前の予定表示に用途を限定する。
- 収支サマリは `result_confirmed` を検知して自動精算・自動反映。手動ボタンはフォールバック。

## 理由

- **精算エンジンを二重実装しない**。`settle_bet`・`parse_race_payouts`・冪等な再計算は #40 で確定済み。結果確定は「着順を `results` に持つか」という **既存テーブルからの派生**で表し、状態カラムや別テーブルを増やさない（「一時的な修正をしない」「シンプル第一」）。
- **netkeiba を二重取得しない**。着順と払戻は同一結果ページに載る。1 レース 1 パス 1 取得に集約し、`post_time` 前・確定済みは取得しない gating でページング規律（ADR 0021/0029・IP ブロック回避）を守る。web ポーリングも確定で止める。
- **SPA の鮮度方針と整合**。`web-spa.md` は「永続化済みデータを表示・結果未確定のレースでのみ最新取得ボタン」。本設計はその「最新取得」を発走後レースに対する自動ポーリングへ拡張するもので philosophy を崩さない（取得 → 保存 → 最新値、が read の一貫方針）。
- **decision-support の一線を越えない**。自動化するのは「結果照合という手作業（着順・払戻の突合と収支反映）」であり、張る/見送り/増額の判断や軸ロック（ADR 0055/0060）には触れない。

### 代替案と棄却理由

- **サーバ側 background sweep（api-server が常駐タスクで自動精算）**: ヘッドレスでも回るが、常駐タスクの lifecycle・停止制御・多重起動制御を api-server に持ち込む。ライブ確認は基本ブラウザを開いて行うため、冪等 API への web ポーリングで要件を満たせる。過剰。棄却。
- **predict-watch に相乗り**: 発走前監視ツールに結果取得を足す案。監視を回している時だけ精算が進み、UI（ブラウザ）単独では反映されない。UI 自動反映という主目的から外れる。棄却。
- **結果確定を専用カラム/専用テーブルで持つ**: `results` の着順存在から派生できるため冗長。二重管理で不整合の芽になる。棄却（派生で表す）。
- **払戻の official 配当を新テーブルに永続化**: UI 要件は「的中○/✗・払戻額」（＝ session の bet 払戻）であり、official 配当そのものの常時表示は不要。取り込みパス内で払戻を消費すれば足り、テーブルを増やさない。YAGNI として棄却（将来必要なら別 Issue）。
- **新規 read エンドポイント `GET /api/results/{date}`**: 分離は綺麗だが web は既に `/api/races`・`/api/live`・`/api/sessions` を引いており、結果フィールドを相乗りさせる方が配線が最小。棄却（既存 DTO 拡張を採用）。

## 影響

- **新規**: use-case `ResultsInteractor::refresh(date)`（結果取り込み＋精算の集約）／repo `upsert_results`（INSERT ON CONFLICT・当日着順の INSERT 経路）／`POST /api/results/{date}:refresh`（rest-controller・router・api-server 配線）／read DTO 3 種への結果フィールド追加（`RaceSummary`・`BoardHorseSchema`＋盤・`LiveRaceViewSchema`）＋ OpenAPI スナップショット更新／web のポーリング＋「終」判定置換＋着順・的中/払戻表示。
- **不変**: `settle_bet`／`parse_race_payouts`／`SettleInteractor` の精算ロジック（冪等・返還優先・#131 全額返還）／確率モデル・EV 層（ADR 0055）・軸ロック（ADR 0060）／`paddock-fetch-results`（過去レース UPDATE・ADR 0015）／`results` スキーマ（列追加なし）。
- **後方互換**: `POST /api/sessions/{date}/results:refresh` は本フローへ委譲するエイリアスとして維持。手動「精算」ボタンはフォールバックとして残す。
- レース結果照合の手作業（netkeiba 直接確認・手動精算）が消え、UI が「発走済み → 着順・的中/払戻・収支」まで自動で追従する。あくまで結果照合の自動化であり、買い方判断（decision-support）は人間側に残る。
- 関連: #40（自動精算エンジン）／#131（全額返還）／#370・#391（終了判定・post_time 一次ソース）／ADR 0015（netkeiba 結果ソース・UPDATE 専用）／0021・0029（ページング/リトライ規律）／0055・0060（EV 層分離・軸ロック）／0064・0066（ライブ EV ビュー）。設計詳細は [docs/specifications/race-result-ingestion.md](../specifications/race-result-ingestion.md)。
