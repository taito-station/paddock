# ADR 0024: fetch/parse のステージ分割と取得状態の DB 管理 (Issue #147)

## ステータス
承認済み（採用）

## コンテキスト
`parse-pdf fetch`（結果 seiseki PDF）は 1 開催ごとに「DL（礼儀ペーシング）→ mutool＋OCR 解析 →
DB 保存」を**同期実行**し、PDF はディスクに書かずメモリ内で parse する。重いのは OCR（CPU バウンド、
~50 秒/開催）で、年単位バックフィルの所要時間の大半を占める。一方 DL はネットワーク律速で、礼儀
ペーシングのため JRA 接続を長時間占有する。

取得（ネットワーク律速）と解析（CPU 律速）を分離すれば、DL を数分で終えて JRA 接続を即解放し、重い
OCR をネットワーク非依存で `-j <コア数>` 並列実行できる（壁時計 ≒ 総OCR ÷ コア数）。

詳細設計は `docs/specifications/fetch-stage-split.md`。

## 決定

### 1. ステージ分割
- **Stage1（DL専用, `fetch --download-only`）**: range 列挙・礼儀ペーシングは現状踏襲し、PDF を
  `pdfs/results/inbox/<JRA ファイル名>.pdf`（`{年}-{回}{場slug}{日}.pdf`、例 `2026-3nakayama6.pdf`）に
  保存するだけ。解析・DB 保存はしない。ファイル名は `MeetingSpec::from_pdf_filename` で source_key に
  復元でき、Stage2 の記録・削除に使う。
- **Stage2（ingest）**: 既存 `ingest`（並列 OCR・`should_pin_ocr`）で inbox を消費し解析・DB 保存。
- 従来形 `fetch`（取得＋即解析）は後方互換として残す。

### 2. 取得状態のライフサイクル 1 テーブル統一（dedup 統一）
現状の 2 系統 dedup（fetch 経路＝`fetch_history`〔成功時のみ〕、ローカル ingest 経路＝`done/` 移動）を、
取得ライフサイクルを表す**単一テーブル**に統一する（既存 `fetch_history` を拡張）。`done/` 移動は廃止し、
ingest 完了時に DB 状態更新＋PDF 削除へ寄せる。

**実装スコープ（#147）**: 本 PR では `fetch_history` に `status`（`downloaded` / `ingested`）のみを
追加した（既存行は ingest 成功ログなので `ingested`）。`fetch_history_contains` は「ingest 済み」に
厳格化し、`fetch_status` / `record_download` を新設して Stage1↔Stage2 の受け渡しを表現する。
`failed` 状態と `http_status` / `attempts` / `last_attempt_at`、および `fetched_at` の `TIMESTAMPTZ`
化（時刻比較・バックオフ用）は、下記論点1とともに **follow-up #170** へ切り出した。

### 3. 失敗の扱い（論点1）— follow-up #170 へ
**当初決定**: JRA はレート制限/IP ブロック時にも 403 を返す（実在 `2026-2tokyo12` が一時 403 になった
実例あり）。「403=不在」で以後スキップすると、ブロックされていた実在開催を永久に取り逃す。よって
`failed` 行は**再試行の入力**であって除外フラグにしない（404→後日リトライ、403→バックオフ再試行、
range の単発 403 は実行内境界・DB は `failed(403)` で次回再試行）。

**#147 実装時の判断（見送り）**: 並列 grid fetch は実在しない開催日の大半が 403/404 になるため、これらを
`failed` 行として記録すると「実在しない開催日のジャンク行」が毎回大量に堆積し、永久に再試行対象として
残る。「未公開でいずれ実在する開催日」と「grid 総当りで永遠に実在しない組合せ」を区別する設計が前提に
なるため、ステージ分割本体とは独立させ **#170** に切り出した。現状は 403/404 を記録せず再取得可能のまま
（#149 の 0 レースと同じ扱い）とし、永久スキップにはしないという論点1の意図は満たしている。
- 成功 → `downloaded`（Stage1）/ `ingested`（Stage2）。

### 4. PDF はデフォルト削除（論点2）
PDF を残す唯一の理由は「**パーサ／抽出ロジック自体を改善したとき過去分を再抽出する**」場合のみ。解析の
充実は OCR 後の構造化データ（DB）から何度でも回せるため PDF は不要。ingest が**完全 parse 完了**（OCR
含む）した時点で削除する。0 レース（parser gap, #149）は完全成功でないため**削除せず保持**し再 ingest
余地を残す。再抽出が必要になった時に意図的な一括再 DL（バックフィル）を行う。

## 理由
- ステージ分割は既存の inbox/ingest 動線・OCR スレッド調整にそのまま乗り、最小の動線変更で最大の壁時計
  短縮（OCR 並列化）を得られる。#146（コンテナ化）で Stage2 を CPU キャップ隔離する布石にもなる。
- 取得状態を 1 テーブルに統一することで「DL 済みだが未解析」を表現でき、ステージ間の受け渡しと重複回避を
  一貫したセマンティクスで扱える。`fetch_history`（成功=ingested）に DL/失敗状態を混ぜる曖昧さを避ける。
- 403 を永久スキップにしない設計は、JRA ブロックという運用現実（#152 の 8.7h ハングと同根の脆さ）に対する
  必須の防御。記録（http/attempts/last_attempt_at）はリトライ判断の入力として使う。

## 影響・トレードオフ
- スキーマ変更（マイグレーション）と取得経路の中核ロジック変更を伴う。#147 では `fetch_history` に
  `status` カラムを追加するのみで、既存行は DEFAULT `'ingested'` で充足（行の移送は不要）。残りの
  ライフサイクル列は #170 で追加する。
- 従来 fetch は PDF をディスクに書かなかったが、Stage1 は inbox に書く（ディスク使用）。ingest 完了で削除
  するため定常的な滞留は無い。
- **本 ADR はスコープを結果 PDF パイプラインに限定**する。OCR の任意化（`--no-ocr`）は最大レバーだが独立
  論点として別 Issue に切り出す。出馬表（entries）側の状態管理も対象外。
- 関連: #146（コンテナ化）、#149（`Empty`/0 レース非記録）、#152/#155（fetcher の timeout/retry・共有化）、
  #170（失敗追跡 `failed`/`http_status`/`attempts` の follow-up）。
