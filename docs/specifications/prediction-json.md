---
# knowledge 規約に基づくメタデータ（docs/knowledge/README.md）。specifications はその場で
# knowledge に昇格（ADR 履歴・相互リンクを壊さないため物理移動しない）。
status: Confirmed
kind: knowledge
doc_class: [D10, D08]
tags: [D10, D08]
updated: "2026-07-21"
---

# 予想 JSON 仕様（ingest-predictions 入力契約）

予想（印・短評・買い目・結果）を DB に保存するための JSON。**DB が正**。予想を作るときはこの
JSON を吐いて `paddock-ingest-predictions` に渡せばよい。閲覧は REST API + SPA（`api-server` /
`web/`）で行う（pad MD 書き出しは ADR 0069 で廃止）。

## 取り込み

```bash
# stdin から
cat pred.json | cargo run -p ingest-predictions
# ファイルから
cargo run -p ingest-predictions -- --input pred.json
# パース・検証のみ（保存しない）
cargo run -p ingest-predictions -- --input pred.json --dry-run
```

単一オブジェクト・オブジェクト配列のどちらでも受け付ける。

## スキーマ

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `date` | string | ○ | 開催日 `YYYY-MM-DD` |
| `venue` | string | ○ | 開催場。日本語(`阪神`)か romaji(`hanshin`) |
| `race_num` | int | ○ | レース番号 |
| `title` | string | | レース名/クラス（H1 に出す） |
| `budget` | int | | 予算（円） |
| `strategy_note` | string | | 買い目の狙い/方針（買い目表の後に出す） |
| `commentary` | string | | 敗因分析等の自由記述（生成 MD 末尾に出す） |
| `horses` | Horse[] | ○ | 各馬（下記） |
| `bets` | Bet[] | | 買い目（下記） |
| `result` | Result | | 結果（答え合わせ後のみ） |

### Horse

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `horse_num` | int | ○ | 馬番 |
| `horse_name` | string | ○ | 馬名 |
| `jockey` | string | | 騎手 |
| `mark` | string | | 印。記号(`◎○▲△☆注`)か slug(`honmei`/`taikou`/`tanana`/`renge`/`hoshi`/`chui`) |
| `win_odds` | number | | 単勝オッズ |
| `popularity` | int | | 人気 |
| `win_prob` | number | | 勝率（**百分率の表示値**。例 `25.4` = 25.4%） |
| `place_prob` | number | | 連対率（百分率） |
| `show_prob` | number | | 複勝率（百分率） |
| `comment` | string | | 短評 |

### Bet

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `bet_type` | string | ○ | 券種（`単勝`/`複勝`/`馬連`/`ワイド`/`馬単`/`3連複`/`3連単` 等、表示ラベル） |
| `combination` | string | ○ | 買い目。arabic 馬番のハイフン連結（`7` / `7-14` / `7-14-13`） |
| `amount` | int | ○ | 金額（円） |

### Result

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `finish` | int[] | | 1〜3 着の馬番（先頭から、最大 3 要素） |
| `recovery_rate` | number | | 回収率（%） |
| `pnl` | int | | 収支（円, 符号付き） |
| `note` | string | | 結果コメント |

## 同定とキー

- レースは `(date, venue, race_num)` で一意（同じキーで再取り込みすると upsert＝冪等）。
- `race_id` は `races`/`race_cards` を `(date, venue, race_num)` で照合できた時だけ自動解決して保持する（未確定・未取込レースでは NULL）。

## 例

```json
{
  "date": "2026-06-13",
  "venue": "hanshin",
  "race_num": 4,
  "title": "3歳未勝利",
  "budget": 10000,
  "strategy_note": "人気軸＋相手広め",
  "horses": [
    {"horse_num":7,"horse_name":"ラパンドール","jockey":"松山","mark":"◎",
     "win_odds":2.4,"popularity":1,"win_prob":25.4,"place_prob":25.4,"show_prob":25.4,
     "comment":"市場・モデルとも単独最上位"}
  ],
  "bets": [
    {"bet_type":"単勝","combination":"7","amount":600},
    {"bet_type":"馬連","combination":"7-14","amount":1000}
  ],
  "result": {"finish":[7,4,13],"recovery_rate":52.1,"pnl":-4790,"note":"印は上位3頭を捕捉"}
}
```

---

## 決定ログ

<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->

### ADR 0025: 予想の横断検索 API (Issue #145) (2026-06-19) — 承認済み

#### コンテキスト

予想を DB に永続化（#144 / `predictions`・`prediction_horses`・`prediction_bets`）した後、蓄積された予想を横断的に探索する導線が無い。予想ビューア（PR #143）は日付 > 開催場 > レースのツリーで 1 件ずつ開くだけで、「あの馬の予想だけ見たい」「印が◎だったレースの的中率」といった軸の探索ができない。

提供形態として「ビューア拡張 / CLI / API」が候補だった。read REST API（#33）は完成済みで Web SPA（#34）の read データ源になる予定、`web-viewer` は MD を読む静的ビューアで DB を読まない。検索を `web-viewer` に足すと「web-viewer の DB 化」という別軸の改修＋将来 #34 SPA と二重投資になる。CLI は最小だが Web 化（#34）への直結性が低い。

#### 決定

- 提供形態は **REST API（`apps/api-server`）の拡張**とする。#33 の rest-controller / api-server / utoipa / エラー封筒・use-case / repository 層構成をそのまま流用し、#34 SPA の read 源として合流させる。
- エンドポイント 3 本を追加する:
  - `GET /api/predictions` … 横断検索・絞り込み（一覧 + `total_count`、`limit`/`offset` ページング）
  - `GET /api/predictions/{prediction_id}` … 個別予想（ビューア相当の全項目）
  - `GET /api/predictions/stats/by-mark` … 印別の的中率（集計の入口 1 本）
- 検索軸: 日付・期間 / 開催場 / 距離 / 芝ダ / 馬名（部分一致・カナ正規化）/ 印 / 的中・不的中。指定軸のみ AND で絞る。
- **距離・芝ダは `races` 結合**で得る。一覧の `distance`/`surface` 表示用に **`races` は常時 `LEFT JOIN`**、距離・芝ダ**フィルタは指定時のみ `WHERE` で絞る**（指定時は `race_id` NULL の未照合予想が NULL 述語で脱落＝実質 INNER 相当）。この脱落は仕様とし OpenAPI 説明文で明示する。race_id 補完は本 Issue の対象外。
- **馬名検索は #50 の資産を 2 経路に分けて流用**: (a) カナ正規化は `HorseName::try_from`（domain 値オブジェクト。内部で `src/domain/src/normalize.rs` の正規化を適用）。(b) 中間一致は既存 `find_matching_horse_names`（`NameMatchRepository`）の `LIKE '%' || $1 || '%' ESCAPE '\'` + `escape_like()` イディオムを `prediction_horses` 向け新規クエリに適用（`escape_like` は private のため `pub(crate)` 化／共通化して流用）。馬名は中間一致のため btree index は効かずフルスキャン（件数小で許容）。analyze/horse は完全一致のため流用するのは正規化のみ。`prediction_horses.horse_name` は predict パイプライン（正規化済みの race_cards / results 由来）から生成されるため、クエリ側正規化のみで部分一致が成立する。取り込み時正規化＋バックフィルは見送る（ロスあり・スコープ拡大）。
- 動的 WHERE は「静的フラグメントのみ `format!`、値は必ず `.bind()`」で組み、`venue`/`surface` は `Venue`/`Surface`、`mark` は OpenAPI enum を slug に固定して検証する。
- **馬名 × 印を併用**した場合は同一馬が両条件を満たすことを要求する（単一 `EXISTS` 内で `horse_name LIKE ... AND mark = ...`）。
- **的中は回収率ベース**で定義: 的中 = `recovery_rate > 0`、不的中 = `finish_1 IS NOT NULL AND COALESCE(recovery_rate,0)=0`、結果未記録 = `hit` フィルタ対象外。買い目と着順の突き合わせは行わず、取り込み済みの `recovery_rate` を正とする。
- **集計は印別的中率 1 本**に限定。印ごとに 1 着率・複勝圏率（`horse_num` と `finish_1/2/3` の照合）を返す。詳細クロス集計は #34 / `analyze` に委ねる。
- **マイグレーション不要**。期間絞り込みは `UNIQUE(date,venue,race_num)` の先頭列 date が効く（最終ソートは ASC/DESC 混在で別ソート段になりうるが件数小で許容）。印は等価比較で `idx_prediction_horses_mark` が有効、馬名は中間一致のため btree（`idx_prediction_horses_name`）は効かずフルスキャン（件数小で許容）、距離・芝ダは `races` の PK（`race_id`）結合で `idx_races_course` は寄与しない。いずれも新規インデックス不要。
- OpenAPI は utoipa コードファーストで拡張し、`docs/api/openapi.json` をスナップショット検証する。

#### 理由

- **API を選ぶ理由**: #33 完成済み資産を最大限再利用でき、#34 SPA の read 源として一直線に合流する。web-viewer 拡張は別軸（DB 化）の改修と #34 との二重投資、CLI は Web 化への直結性が低い。
- **races 結合で距離・芝ダ**: `predictions` に距離・芝ダを非正規化複製すると取り込み・整合保守が増える。`races` 結合 + 既存 `idx_races_course` で十分。NULL 除外は明示仕様にする。
- **回収率ベースの的中**: 買い目×着順の突き合わせは過剰実装で、取り込み時算出済みの `recovery_rate` と二重定義になりうる。保存値を正とするのが最小かつ無矛盾。
- **集計を 1 本に絞る**: Issue は「集計の入口」を求めており、詳細分析は #34 / `analyze` と整理済み。最小形から始める方針に合致。
- **マイグレーション回避**: 既存インデックスで要件を満たすため、スキーマ変更のコスト・リスクを負わない。

#### 影響

- use-case interactor を 3 つ追加（`search_predictions` / `prediction_detail` / `prediction_mark_stats`）。`PadPredictionRepository`（use-case トレイト）にも read メソッドを 3 つ追加するが、層ごとに名前を分ける: interactor `search_predictions` → repo `search_predictions`、interactor `prediction_detail` → repo `find_pad_prediction_by_id`（既存 `find_pad_prediction` は `(date,venue,race_num)` キーのため PK 取得版を新設）、interactor `prediction_mark_stats` → repo `prediction_mark_stats`。Postgres 実装（`rdb-gateway`）と、トレイトを実装する全ダミー/モックの網羅にコンパイラが追従を要求する。
- rest-controller に handler / schema / router を追加し、`ApiDoc` の paths/components が増える → `docs/api/openapi.json` 再生成が必要（スナップショットテストで強制）。
- 距離・芝ダ絞り込みは `race_id` 解決済みの予想に限られる。SPA はこの制約を UI 上で示す前提（未照合分の取りこぼし）。
- 馬名検索は表記ゆれ（取り込み時非正規化）に理論上弱い。実害が観測されれば取り込み時正規化＋バックフィルを別 Issue で対応する。
- read 専用・件数が小さいため新規インデックスは張らない。将来件数増・遅延が出れば `EXPLAIN ANALYZE` で確認のうえ索引を別途追加する。
- 採番当時 `0022` が 2 ファイル重複していた（`0022-rest-api-read-server.md` / `0022-shared-jra-fetcher-crate.md`）ため本 ADR は連番末尾 `0025` で採番した。重複は後に是正済み（後発の `jra-fetcher 集約` を ADR `0029` にリナンバー、2026-06-20）。

### ADR 0032: gen_predictions.py の買い目はモデル確率ベースで常に生成する (2026-06-21) — 承認済み

#### コンテキスト

`gen_predictions.py`（朝の一括予想生成スクリプト）が生成する prediction JSON の
`bets` フィールドが空で、Obsidian web-viewer の「買い目」欄が常に空白になっていた（#201）。

買い目生成には2つの選択肢があった。

- **案 A**: モデル確率だけから常に生成する（EV フィルタなし）
- **案 B**: ワイド/馬連/3連複オッズを取得・ROI を計算し、+EV レースだけ付ける

#### 決定

**案 A（モデル確率ベース、EV フィルタなし）**を採用する。

`build_bets`（`live_ev.py` から import）を用い、各レースのモデル勝率から
確率重み配分で買い目を算出して `bets` に載せる。予算は ¥5,000/レース 固定。

#### 理由

- `gen_predictions.py` は朝の一括実行を想定しており、その時点でワイドオッズ（netkeiba
  type=5）や馬連/3連複オッズ（発売前または未取得）が揃っていないケースが多い。
- `build_bets` の入力はモデル勝率のみで、ここから組合せ・金額を確定できる。
  ROI 計算（`race_roi`）は別の関心事であり、EV 判断は引き続き `refresh_ev.sh` が担う。
- 案 B は生成に netkeiba fetch が必要で、gen_predictions.py を遅くし・外部依存を増やす。
  既存の役割分担（gen_predictions = 本命確認 + 買い目案、refresh_ev = EV 判定）を崩さない。

#### 影響

- `gen_predictions.py` が常に `bets` を含む prediction JSON を出力するようになる。
  Obsidian web-viewer に買い目案が表示される。
- `bets` はモデル確率時点の「デフォルト買い目案」であり、ライブ EV で −EV と判定された
  レースでも表示される。レース選択（張る/見送り）は引き続きライブ EV の ROI ≥ 100% で行う。
- `live_ev.py` のロジック（`build_bets`/`is_konsen`/`band_of` 等）を変更した場合、
  gen_predictions.py も同じ変更の影響を受ける（同一モジュールを import しているため自動的に追従）。
- `ingest-predictions` は `bets` フィールドを配列として受け取る。空配列（障害レース等）は正常値
  として扱われ、ingest 側でエラーにはならない（フィールドが存在しない場合の動作は未規定のため、
  gen_predictions.py は常に `bets: []` を含めて出力する）。
