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

新ユースケース `ResultsInteractor::refresh(date, force)` を追加する（`force` は §2 書き込み口参照・既定 false）。対象は **開催日のレースのうち `post_time` を過ぎ、かつ未確定**のもの（発走前・確定済みは netkeiba を叩かずスキップ。`force=true` は post_time gating を緩和）。

1. 各対象レースにつき netkeiba 結果ページ（`race/result.html`）を **1 回だけ取得**し、同一 HTML から着順（`parse_race_result`）と払戻（`parse_race_payouts`）を **両方**パースする。結果ページに双方が載るため往復を二重化しない。ただし既存 `fetch_race_result` と `fetch_race_payouts` は各々が独立に GET する 2 メソッドのため、**HTML を 1 回取得して両パーサに渡す新 scraper メソッド**（例 `fetch_race_result_page`）を追加する（実装点）。取得の pacing・リトライ規律は ADR 0021（HTTP タイムアウト＋リトライ）・0029（fetcher 集約）＋運用 pacing（CLAUDE.md）に準拠。
2. **`races` 行の担保**: `results.race_id` は `races(race_id)` への FK だが、当日フロー（`paddock-fetch-card` → `card/ingest.rs`）は `race_cards`/`horse_entries`/`race_odds` のみ保存し `races` 行を作らない（`races` の INSERT は PDF ingest 経路＝`save_race` のみ）。よって着順 upsert の前に、`race_cards` から `races` 行を派生 upsert して FK を満たす。
3. 着順を `results` へ **upsert**（INSERT ... ON CONFLICT `(race_id, horse_num)` DO UPDATE、`source='netkeiba'`）。当日は着順行が無いため **INSERT 経路が新設点**（既存 `update_results` は UPDATE 専用のため別メソッド `upsert_results` を追加）。`results` の NOT NULL 列 `gate_num`/`horse_name` は **`ResultRow` に含まれない**（結果ページからは取得しない）ため、**常に** `race_cards`（出馬表）を `(race_id, horse_num)` で引いて補完する。`race_cards` が無いレース（出馬表未取得）は補完不能のため当該レースを pending 据え置きにする。
4. 同一パスでその払戻を使い、**セッションがあれば** `settle_bet` で精算する（払戻はこのパスで取得済みのものを in-memory で渡し、`settle_session` のような netkeiba 再取得をしない）。精算は従来どおり冪等・pending 据え置き・全額返還（#131）を踏襲する。
5. `SettleReport` を拡張した `RefreshReport`（精算サマリ＋新規確定レース数・確定 `race_id` 一覧）を返す。

**中止レースの確定縮退**: 開催中止で netkeiba 結果ページに成績表が生成されない場合、`parse_race_result` は空を返し着順行が入らない（既存 `settle_session` も同状況を pending 据え置きとする既知制約）。この場合の確定判定は「`post_time` から一定時間（既定 N 分）経過しても成績表が生成されない」タイムアウトで確定扱いにするか、手動フォールバックに委ねる（自動では延々 pending にしない）。

**結果確定フラグ `result_confirmed` は派生値**とし、専用カラムを増やさない（シンプル第一）。定義: 当該 `race_id` の `results` に **`finishing_position IS NOT NULL` の行が 1 つ以上**存在すること（着順が取り込まれた＝確定）。`ResultStatus` は `finished`/`scratched`/`cancelled`/`did_not_finish` の 4 値で、一部の非完走行だけが landed した中間状態を誤って確定としないため、単に `status <> 'finished'` では判定しない。全馬取消/中止（着順 NULL）は前述のタイムアウト縮退で確定扱いにする。これにより **賭けていない・スキップしたレースも**「終」を確定でき、着順表示もできる。

### 2. API への結果公開（read）

既存 read の DTO に結果フィールドを足す（新規 read エンドポイントは作らず、web の既存クエリに相乗り）。

- `GET /api/races`（`RaceSummary`）: `result_confirmed: bool` と `finish_order: [{position, horse_num, horse_name}]`（上位 3）を追加。ライブ一覧の「終」バッジ・着順表示の一次ソース。
- `GET /api/races/{race_id}/board`（`BoardHorseSchema`）: `finishing_position: Option<u32>` を各馬に、`result_confirmed: bool` を盤に追加。1 レース盤の結果反映。
- `GET /api/live/{date}`（`LiveRaceViewSchema`）: `result_confirmed: bool` を追加（「⚫終」を推定から確定へ置換）。的中/払戻は既存 `GET /api/sessions/{date}` の `bets[].payout` から web が算出する（別ソースを増やさない）。
- 書き込み口: `POST /api/results/{date}:refresh`（`?force=` 付き）を新設し `ResultsInteractor::refresh(date, force)` を起動。自動ポーリングは `force=false`（post_time gating あり）、手動フォールバックは `force=true`（gating 緩和で post_time 欠損レースも救済）。既存 `POST /api/sessions/{date}/results:refresh` は本フローへ委譲するエイリアスに変更し、**同じく `?force=` を受理・転送する**（手動ボタンはこのエイリアス経由で `force=true` を渡す。`force` 既定 false のため旧 CLI 呼び出しは無指定で従来挙動）。**レスポンス互換は保つが、着順の `results` upsert という副作用が新たに加わる**（純粋な後方互換ではない点を明示）。
- **OpenAPI を一級成果物**とする（utoipa コードファースト＋`openapi.json` スナップショット更新・検証を DoD 化）。

### 3. UI 自動反映（web）

- **自動精算トリガーは web ポーリング駆動**。ライブ一覧／収支サマリで、**`post_time` を過ぎ、かつ未確定のレースが 1 件以上残る間だけ** `POST /api/results/{date}:refresh` を 30–60 秒間隔で叩く（当日でも全レースが発走前なら対象 0 でポーリングしない・空振りさせない）。`ResultsInteractor` は冪等なので何度叩いても安全。**全レース確定でポーリング停止**（netkeiba への無駄打ちを止める）。手動「精算」ボタンは**フォールバックとして残す**。
- **サーバ側の取得多重化ガード**: ポーリングは netkeiba を実取得する write API を叩くため、複数タブ/複数クライアントが同一 `date` を同時ポーリングすると同じ未確定レースへの取得が多重化し IP ブロック（本 PJ の最重要運用リスク）を招く。冪等は「結果の二重加算防止」は担保するが「取得多重化防止」はしないため、サーバ側に in-flight ロック or 直近取得の debounce（同一レースを N 秒以内は再取得しない）を設ける。
- ライブ一覧の発走済み行に **的中○/✗・払戻額**（session `bets[].payout` 由来）と **着順**（`finish_order` 由来）を表示。「⚫終」は `result_confirmed` で判定（`post_time` 推定を置換）。`post_time` は発走前の予定表示に用途を限定する。
- 収支サマリは `result_confirmed` を検知して自動精算・自動反映。手動ボタンはフォールバック。

## 理由

- **精算エンジンを二重実装しない**。`settle_bet`・`parse_race_payouts`・冪等な再計算は #40 で確定済み。結果確定は「着順を `results` に持つか」という **既存テーブルからの派生**で表し、状態カラムや別テーブルを増やさない（「一時的な修正をしない」「シンプル第一」）。
- **netkeiba を二重取得しない**。着順と払戻は同一結果ページに載る。1 レース 1 パス 1 取得に集約し、`post_time` 前・確定済みは取得しない gating と、取得の pacing・リトライ規律（ADR 0021 タイムアウト＋リトライ・0029 fetcher 集約、運用 pacing は CLAUDE.md・IP ブロック回避）を守る。web ポーリングも確定で止める。
- **SPA 鮮度方針は明示的に改訂する（崩さない、で流さない）**。`web-spa.md` は現状「SPA は自動ポーリングしない・更新は明示的ユーザー操作」「再現性重視・自動更新なし」とし、ポーリングを非対象に挙げている。本設計はこれと正面から衝突するため、**「当日・未確定レースに限り自動ポーリングを許可する」と鮮度方針を明示的に上書きする決定**とする（過去日・確定済みは従来どおり自動更新しない）。取得 → 保存 → 最新値という read の一貫方針は保つ。`web-spa.md` の当該記述の改訂を影響範囲に含める。
- **decision-support の一線を越えない**。自動化するのは「結果照合という手作業（着順・払戻の突合と収支反映）」であり、張る/見送り/増額の判断や軸ロック（ADR 0055/0060）には触れない。

### 代替案と棄却理由

- **サーバ側 background sweep（api-server が常駐タスクで自動精算）**: ヘッドレスでも回るが、常駐タスクの lifecycle・停止制御・多重起動制御を api-server に持ち込む。ライブ確認は基本ブラウザを開いて行うため、冪等 API への web ポーリングで要件を満たせる。過剰。棄却。
- **predict-watch に相乗り**: 発走前監視ツールに結果取得を足す案。監視を回している時だけ精算が進み、UI（ブラウザ）単独では反映されない。UI 自動反映という主目的から外れる。棄却。
- **結果確定を専用カラム/専用テーブルで持つ**: `results` の着順存在から派生できるため冗長。二重管理で不整合の芽になる。棄却（派生で表す）。
- **払戻の official 配当を新テーブルに永続化**: UI 要件は「的中○/✗・払戻額」（＝ session の bet 払戻）であり、official 配当そのものの常時表示は不要。取り込みパス内で払戻を消費すれば足り、テーブルを増やさない。YAGNI として棄却（将来必要なら別 Issue）。
- **新規 read エンドポイント `GET /api/results/{date}`**: 分離は綺麗だが web は既に `/api/races`・`/api/live`・`/api/sessions` を引いており、結果フィールドを相乗りさせる方が配線が最小。棄却（既存 DTO 拡張を採用）。

## 影響

- **新規**: use-case `ResultsInteractor::refresh(date, force)`（結果取り込み＋精算の集約・`RefreshReport` 返却）／HTML を 1 回取得して着順・払戻を両パースする scraper メソッド（既存 2 メソッドの二重 GET を避ける）／repo `upsert_results`（INSERT ON CONFLICT・当日着順の INSERT 経路）＋ `race_cards` からの `races` 行派生 upsert（FK 担保）／`POST /api/results/{date}:refresh`（rest-controller・router・api-server 配線）＋サーバ側の取得 debounce/in-flight ガード／read DTO 3 種への結果フィールド追加（`RaceSummary`・`BoardHorseSchema`＋盤・`LiveRaceViewSchema`）＋ OpenAPI スナップショット更新／web のポーリング＋「終」判定置換＋着順・的中/払戻表示／`web-spa.md` 鮮度方針の改訂（当日・未確定に限り自動ポーリング許可）。
- **不変**: `settle_bet`／`parse_race_payouts`／`SettleInteractor` の精算ロジック（冪等・返還優先・#131 全額返還）／確率モデル・EV 層（ADR 0055）・軸ロック（ADR 0060）／`paddock-fetch-results`（過去レース UPDATE・ADR 0015）／`results` スキーマ（列追加なし）。
- **後方互換**: `POST /api/sessions/{date}/results:refresh` は本フローへ委譲するエイリアスとして維持。手動「精算」ボタンはフォールバックとして残す。
- レース結果照合の手作業（netkeiba 直接確認・手動精算）が消え、UI が「発走済み → 着順・的中/払戻・収支」まで自動で追従する。あくまで結果照合の自動化であり、買い方判断（decision-support）は人間側に残る。
- 関連: #40（自動精算エンジン）／#131（全額返還）／#370・#391（終了判定・post_time 一次ソース）／ADR 0015（netkeiba 結果ソース・UPDATE 専用）／0021（HTTP タイムアウト＋リトライ）・0029（fetcher 集約）／0055・0060（EV 層分離・軸ロック）／0064・0066（ライブ EV ビュー）。設計詳細は [docs/specifications/race-result-ingestion.md](../specifications/race-result-ingestion.md)。
