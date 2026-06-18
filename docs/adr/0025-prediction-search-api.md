# ADR 0025: 予想の横断検索 API (Issue #145)

## ステータス
承認済み

## コンテキスト

予想を DB に永続化（#144 / `predictions`・`prediction_horses`・`prediction_bets`）した後、蓄積された予想を横断的に探索する導線が無い。予想ビューア（PR #143）は日付 > 開催場 > レースのツリーで 1 件ずつ開くだけで、「あの馬の予想だけ見たい」「印が◎だったレースの的中率」といった軸の探索ができない。

提供形態として「ビューア拡張 / CLI / API」が候補だった。read REST API（#33）は完成済みで Web SPA（#34）の read データ源になる予定、`web-viewer` は MD を読む静的ビューアで DB を読まない。検索を `web-viewer` に足すと「web-viewer の DB 化」という別軸の改修＋将来 #34 SPA と二重投資になる。CLI は最小だが Web 化（#34）への直結性が低い。

## 決定

- 提供形態は **REST API（`apps/api-server`）の拡張**とする。#33 の rest-controller / api-server / utoipa / エラー封筒・use-case / repository 層構成をそのまま流用し、#34 SPA の read 源として合流させる。
- エンドポイント 3 本を追加する:
  - `GET /api/predictions` … 横断検索・絞り込み（一覧 + `total_count`、`limit`/`offset` ページング）
  - `GET /api/predictions/{prediction_id}` … 個別予想（ビューア相当の全項目）
  - `GET /api/predictions/stats/by-mark` … 印別の的中率（集計の入口 1 本）
- 検索軸: 日付・期間 / 開催場 / 距離 / 芝ダ / 馬名（部分一致・カナ正規化）/ 印 / 的中・不的中。指定軸のみ AND で絞る。
- **距離・芝ダは `races` 結合**で得る。一覧の `distance`/`surface` 表示用に **`races` は常時 `LEFT JOIN`**、距離・芝ダ**フィルタは指定時のみ `WHERE` で絞る**（指定時は `race_id` NULL の未照合予想が NULL 述語で脱落＝実質 INNER 相当）。この脱落は仕様とし OpenAPI 説明文で明示する。race_id 補完は本 Issue の対象外。
- **馬名検索は #50 の資産を 2 経路に分けて流用**: (a) カナ正規化は `HorseName::try_from`（domain 値オブジェクト。内部で `domain/src/normalize.rs` の正規化を適用）。(b) 中間一致は既存 `find_matching_horse_names`（`NameMatchRepository`）の `LIKE '%' || $1 || '%' ESCAPE '\'` + `escape_like()` イディオムを `prediction_horses` 向け新規クエリに適用（`escape_like` は private のため `pub(crate)` 化／共通化して流用）。馬名は中間一致のため btree index は効かずフルスキャン（件数小で許容）。analyze/horse は完全一致のため流用するのは正規化のみ。`prediction_horses.horse_name` は predict パイプライン（正規化済みの race_cards / results 由来）から生成されるため、クエリ側正規化のみで部分一致が成立する。取り込み時正規化＋バックフィルは見送る（ロスあり・スコープ拡大）。
- 動的 WHERE は「静的フラグメントのみ `format!`、値は必ず `.bind()`」で組み、`venue`/`surface` は `Venue`/`Surface`、`mark` は OpenAPI enum を slug に固定して検証する。
- **馬名 × 印を併用**した場合は同一馬が両条件を満たすことを要求する（単一 `EXISTS` 内で `horse_name LIKE ... AND mark = ...`）。
- **的中は回収率ベース**で定義: 的中 = `recovery_rate > 0`、不的中 = `finish_1 IS NOT NULL AND COALESCE(recovery_rate,0)=0`、結果未記録 = `hit` フィルタ対象外。買い目と着順の突き合わせは行わず、取り込み済みの `recovery_rate` を正とする。
- **集計は印別的中率 1 本**に限定。印ごとに 1 着率・複勝圏率（`horse_num` と `finish_1/2/3` の照合）を返す。詳細クロス集計は #34 / `analyze` に委ねる。
- **マイグレーション不要**。期間・ソートは `UNIQUE(date,venue,race_num)` 複合インデックス、馬名・印は `idx_prediction_horses_{name,mark}`、距離・芝ダは `idx_races_course` で賄える。
- OpenAPI は utoipa コードファーストで拡張し、`docs/api/openapi.json` をスナップショット検証する。

## 理由

- **API を選ぶ理由**: #33 完成済み資産を最大限再利用でき、#34 SPA の read 源として一直線に合流する。web-viewer 拡張は別軸（DB 化）の改修と #34 との二重投資、CLI は Web 化への直結性が低い。
- **races 結合で距離・芝ダ**: `predictions` に距離・芝ダを非正規化複製すると取り込み・整合保守が増える。`races` 結合 + 既存 `idx_races_course` で十分。NULL 除外は明示仕様にする。
- **回収率ベースの的中**: 買い目×着順の突き合わせは過剰実装で、取り込み時算出済みの `recovery_rate` と二重定義になりうる。保存値を正とするのが最小かつ無矛盾。
- **集計を 1 本に絞る**: Issue は「集計の入口」を求めており、詳細分析は #34 / `analyze` と整理済み。最小形から始める方針に合致。
- **マイグレーション回避**: 既存インデックスで要件を満たすため、スキーマ変更のコスト・リスクを負わない。

## 影響

- use-case interactor を 3 つ追加（`search_predictions` / `prediction_detail` / `prediction_mark_stats`）。`PadPredictionRepository`（use-case トレイト）にも read メソッドを 3 つ追加するが、層ごとに名前を分ける: interactor `search_predictions` → repo `search_predictions`、interactor `prediction_detail` → repo `find_pad_prediction_by_id`（既存 `find_pad_prediction` は `(date,venue,race_num)` キーのため PK 取得版を新設）、interactor `prediction_mark_stats` → repo `prediction_mark_stats`。Postgres 実装（`rdb-gateway`）と、トレイトを実装する全ダミー/モックの網羅にコンパイラが追従を要求する。
- rest-controller に handler / schema / router を追加し、`ApiDoc` の paths/components が増える → `docs/api/openapi.json` 再生成が必要（スナップショットテストで強制）。
- 距離・芝ダ絞り込みは `race_id` 解決済みの予想に限られる。SPA はこの制約を UI 上で示す前提（未照合分の取りこぼし）。
- 馬名検索は表記ゆれ（取り込み時非正規化）に理論上弱い。実害が観測されれば取り込み時正規化＋バックフィルを別 Issue で対応する。
- read 専用・件数が小さいため新規インデックスは張らない。将来件数増・遅延が出れば `EXPLAIN ANALYZE` で確認のうえ索引を別途追加する。
- 既存 ADR は main に `0022` が 2 ファイル重複している（`0022-rest-api-read-server.md` / `0022-shared-jra-fetcher-crate.md`）。本 ADR は連番末尾 `0025` で採番する（`0022` 重複の是正は本 Issue のスコープ外）。
