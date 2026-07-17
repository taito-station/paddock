# paddock

JRA 公式のレース成績 PDF をパースして Postgres に蓄積し、その実績から各馬の勝率・連対率・複勝率を
推定して期待値（EV/ROI）にもとづき買い目まで出す競馬予想 CLI 群（Rust 製）。「成績を貯める」基盤に加え、
「貯めた成績で予想する」レイヤ（確率推定・買い目推奨・セッション収支記録・backtest 検証）を持つ。
当日の出馬表・オッズ・近走・確定結果は netkeiba からも取得でき、発走直前の EV/ROI 監視やオッズ
時系列の収集、REST API + Web 盤面（React）でのブラウザ表示までカバーする。

## 何ができるか

**集計（analyze）**

- 馬の特性集計: 芝/ダート、距離帯、枠順（内/中/外）、馬場状態ごとの勝率・連対率
- コースの特性集計: 競馬場 × 距離 × 芝ダ で、枠順別の勝率・連対率
- 騎手・調教師の基本スタッツ: 全体／芝ダ／枠順別の勝率・連対率

**予想（analyze predict / predict）**

- 1 レースの win/place/show 確率推定（馬・騎手・調教師・コース・馬場・前走フォーム・斤量などの
  シグナルを重み付き平均し、ベイズ縮約・市場オッズブレンドで補正）— `analyze predict` / `predict` 共通
- 期待値にもとづく買い目推奨（ワイド・馬連・三連複の◎軸ながし、券種予算を 100 円単位で均等配分）— `predict` のみ
- 1 開催日を対話的に予想して賭け金・払い戻し・収支を記録するセッション — `predict`
- 日付範囲での backtest 検証（的中率・回収率・Brier・LogLoss）— `analyze backtest`

**監視・収集（predict-watch / odds-collect）**

- 発走前レースの定期スキャンと、発走直前オッズでの EV/ROI 再計算・通知 — `predict-watch`
- 全レースの単複オッズ時系列を終日収集 — `odds-collect`

**取得（parse-pdf / parse-entries / fetch-\*）**

- JRA 成績 PDF・出馬表 PDF の取り込み（ローカル / URL / 開催指定の自動取得）
- netkeiba からの当日出馬表・オッズ・近走・確定結果の取得

**保存・閲覧（ingest-predictions / web-viewer / api-server + web）**

- 予想（印・短評・買い目・結果）の DB 保存と pad MD 生成 — `ingest-predictions`
- 予想 MD のブラウザ閲覧 — `web-viewer`
- REST API（actix-web）+ React SPA によるライブ盤面 — `api-server` / `web/`

## セットアップ

### 必要環境

- macOS (Apple Silicon / Intel)。Linux でも動作見込み。
- Rust ツールチェイン: `rust-toolchain.toml` で `1.97.1` 固定
- **mupdf-tools** (`mutool` バイナリ): PDF テキスト抽出に使用
  ```bash
  brew install mupdf-tools
  ```
  JRA の PDF は埋め込みフォントを使うため、純 Rust の `pdf-extract` では文字化けする。`mutool` をサブプロセス経由で呼び出して回避している。
- **tesseract** + **tesseract-lang**（jpn パック）: 成績 PDF の着順検証と、mutool が取りこぼした行の
  OCR 補完に使う。`parse-pdf` 起動時に preflight チェックが走り、未インストール／jpn パック未導入なら
  その場でエラー終了する。
  ```bash
  brew install tesseract tesseract-lang   # 日本語パック含む
  tesseract --list-langs                  # jpn が表示されることを確認
  ```
  斤量・人気・調教師・騎手は OCR ではなく mutool のテキスト／座標／単勝オッズ順位から決定的に取得する。
  OCR はこれらに対しては冗長だが、着順は OCR 由来の値で上書きする余地があるため tesseract は必須。

### ビルド

```bash
cargo build --release
```

### Git フック

push 前に CI 相当の高速チェック（`cargo fmt --all --check` / `cargo clippy --locked --workspace --all-targets -- -D warnings` / ADR 番号の重複検出）を走らせる pre-push フックをリポジトリ管理下（`scripts/git-hooks/`）に置いている。`core.hooksPath` はローカル設定でコミットされないため、**clone・並走 clone ごとに一度だけ配線する**:

```bash
scripts/install-git-hooks.sh
```

- 配線後は `core.hooksPath` のみが参照され、旧 `.git/hooks/pre-push` は無効化される（二重実行なし）。
- 相対 `core.hooksPath` の性質上、`scripts/git-hooks/` を含まないブランチ/worktree をチェックアウトした状態ではフックは走らない（git が「フック無し」として扱う）。
- 緊急時の意図的バイパスは `git push --no-verify`（規約上、明示意図があるときのみ）。

### DB（Postgres）

- バックエンドは **Postgres**。`deployments/compose.yaml` で起動する
  （`docker compose -f deployments/compose.yaml up -d postgres`）。compose のサービスは 4 つ:
  - `postgres` … DB 本体（`127.0.0.1:5432`、named volume で永続化）
  - `api` … REST API サーバの常駐実行（`127.0.0.1:8080`。「REST API とライブ盤面」参照）
  - `web` … Web SPA を nginx で配信（`127.0.0.1:8081`。`/api` を `api` へリバースプロキシ）
  - `importer` … 成績取り込みの隔離実行（「運用」参照）
- 接続先は環境変数 `PADDOCK_DB_URL`（既定 `postgres://paddock:paddock@localhost:5432/paddock`）。
  `.env.example` を `.env` にコピーして調整する。
- スキーマは各アプリ起動時に自動マイグレート（`deployments/db/migrations/`、sqlx）。接続プールは `max_connections=5`。
- テスト（`rdb-gateway` の統合テスト等）は `#[sqlx::test]` がテストごとに一時 database を自動作成・破棄する。
  実行時に PG を指す `DATABASE_URL` が要る（例: `DATABASE_URL=postgres://paddock:paddock@localhost:5432/paddock cargo test`）。

## データを取り込む

### 成績 PDF（parse-pdf）

URL から:
```bash
cargo run -p parse-pdf -- https://www.jra.go.jp/datafile/seiseki/report/2026/2026-3nakayama6.pdf
```

ローカルファイルから:
```bash
cargo run -p parse-pdf -- pdfs/results/inbox/2026-3nakayama6.pdf
```

複数ファイルを並列で取り込む（既定は CPU コア数まで同時実行）:
```bash
cargo run -p parse-pdf -- pdfs/results/inbox/*.pdf
cargo run -p parse-pdf -- -j 4 pdfs/results/inbox/*.pdf   # 並列度を明示
```

`pdfs/results/inbox/` 配下のファイルを引数にした場合、取り込みが成功した PDF は `pdfs/results/done/` へ自動的に移動される（未取り込みファイルが一目で分かるようにするため）。`samples/` などインボックス外のパスは移動されない。複数ファイル指定時、1 件でも失敗があれば終了コードが非 0 になる（成功分の取り込みと移動は維持される）。

なお、明示的に `ingest` サブコマンドを書いても同じ動作になる（`cargo run -p parse-pdf -- ingest pdfs/inbox/*.pdf`）。引数なしの従来呼び出しはデフォルトで `ingest` として扱われる。

### 開催指定で JRA から自動取得（parse-pdf fetch）

完全な URL を組み立てなくても、**年・競馬場・開催回・日次**を指定すれば該当開催の成績 PDF を JRA から取得して取り込める。

```bash
cargo run -p parse-pdf -- fetch --year 2026 --venue nakayama --round 3 --day 6
cargo run -p parse-pdf -- fetch --year 2026 --venue 中山 --round 3 --day 6   # 競馬場は日本語名でも可
```

- 取得した PDF は `https://www.jra.go.jp/datafile/seiseki/report/{年}/{年}-{回}{競馬場}{日}.pdf` から取得し、**メモリ上でパース → DB 保存**する（ローカルには保存しない）。
- 取り込みに成功した開催は `fetch_history` テーブルに記録され、同じ開催を再指定しても**取得・取り込みをスキップ**する（排他制御）。再取得したい場合は `--force` を付ける。
- 指定した開催の PDF がまだ公開されていない（HTTP 404）場合は `not found` として終了コード非 0 になり、履歴には記録されない（公開後に再取得できる）。

#### 範囲指定（まとめて取得）

末尾の引数を省くほど取得範囲が広がる。存在しない日次・開催は自動的に打ち切られる。

```bash
cargo run -p parse-pdf -- fetch --year 2026 --venue nakayama --round 3   # 3回中山の全日次
cargo run -p parse-pdf -- fetch --year 2026 --venue nakayama            # 中山の全開催回×全日次
cargo run -p parse-pdf -- fetch --year 2026                             # その年の全場×全回×全日
```

- 存在確認は「URL 構築＋GET」。JRA は未公開・非存在の成績 PDF を **404 または 403** で返す（公開予定日は 404、開催回・日次を超えた範囲や非開催の競馬場は 403）。どちらも「PDF 無し」として扱い、エラーにはしない。
- 既に `fetch_history` にある開催は**スキップ**されるため、再実行すると差分のみ取得できる（定期実行と相性が良い）。`--force` で全件再取得。
- 最後に `ingested / skipped / not-found / failed` の件数サマリを表示する。途中でネットワーク等の失敗があっても列挙は継続し、`failed > 0` のときのみ終了コード非 0。

##### 並列取得（`-j` / `--parallel`）

OCR が律速（1 開催あたり数十秒〜数分）なので、過去分の一括取り込みは並列化すると大幅に速い。

```bash
cargo run --release -p parse-pdf -- fetch --year 2025 -j 8   # 8 並列で 1 年分を取り込み
```

- 既定の並列度は **CPU コア数**。`-j 1` は逐次・404/403 境界探索になり、`--interval`（既定 1 秒のリクエスト間ウェイト）を尊重する。
- `-j 2` 以上は候補グリッド（場 × 回 × 日）を全列挙して並列取得する（非存在は 404/403 として集計）。
- `fetch --year YYYY`（全場・全回・全日）は最大で `場数 × 8(回) × 14(日)` ≒ **1,000 件超**の GET を JRA に発行しうる。同時実行数は CPU コア数で上限管理され、後段 OCR が律速になるため実効レートは穏当だが、JRA は第三者の公開サーバなので礼節に留意する。
- **`--max-rps`**: JRA への秒間リクエスト数の上限。実際に GET する取得だけを間引く（`fetch_history` ヒットのスキップは対象外なので再実行は遅くならない）。`-j 1` でも作用し、`--interval` とは大きい方が支配的になる。未指定なら無制限。
  ```bash
  cargo run --release -p parse-pdf -- fetch --year 2025 -j 8 --max-rps 2   # JRA へは最大 ~2 req/s
  ```
- 礼節を最優先するなら `-j 1`（逐次・1 秒間隔・境界での早期打ち切り）に戻せば総アクセスも最小化できる。
- 並列時は OCR を 1 プロセス 1 スレッドに固定する（`OMP_THREAD_LIMIT=1` / `OMP_NUM_THREADS=1` を自動設定）。

### 抽出ロジック

抽出は **mutool テキスト/座標抽出 + OCR 補完** のハイブリッド方式で動作する（モード切替なし）。

1. 起動時に `tesseract` バイナリと jpn 言語パックを preflight チェックし、欠けていれば即エラー終了する
2. `mutool draw -F text` で PDF テキストを抽出し、土台となる Race / 結果テーブルを構築する
3. `mutool draw -F stext.json` の座標索引から騎手・調教師・斤量を確定し、人気は単勝オッズの昇順順位から
   算出する（いずれも決定的・OCR 非依存）
4. PDF を PNG 化して OCR をかけ、着順は OCR 抽出が「1〜頭数の完全集合の半分以上を占める」場合のみ
   上書き採用し、そうでなければ mutool の行順 fallback を使う。斤量・調教師などはステップ 3 で
   確定済みのため OCR には依存しないが、ステップ 3 で値が埋まらなかった行があればこの OCR 結果で
   補完する（通常は埋まっているので補完は稀）

進捗は `RUST_LOG=info` で OCR 開始・終了・所要時間が source 別に表示される:

```bash
RUST_LOG=info cargo run -p parse-pdf -- pdfs/results/inbox/2026-3nakayama6.pdf
# INFO ingest{source=pdfs/results/inbox/2026-3nakayama6.pdf}: ocr starting race_count=12 bytes=696311
# INFO ingest{source=pdfs/results/inbox/2026-3nakayama6.pdf}: ocr extracted, applying merge pages=7 elapsed_ms=55114
# INFO ingest{source=pdfs/results/inbox/2026-3nakayama6.pdf}: ocr merge complete
```

JRA PDF は複数レースを 1 ページに収める形式があるため `pages` は `race_count` と一致しないことが多い。

取り込み後の確認:
```bash
psql "$PADDOCK_DB_URL" -c "SELECT race_id, race_num, surface, distance FROM races ORDER BY race_id;"
```

### 出馬表 PDF（parse-entries）

JRA 出馬表 PDF（`N回VENUE日出馬表.pdf`）から枠番・馬番・馬名・騎手を取り込む。

```bash
cargo run -p parse-entries -- pdfs/entries/inbox/20260419-03nakayama08.pdf
# ingested: 12 race card(s), 162 horse entry/entries from ...
# moved: pdfs/entries/inbox/... -> pdfs/entries/done/...
```

`pdfs/entries/inbox/` に置いた PDF は取り込み成功後に `pdfs/entries/done/` へ自動移動する。

取り込み後の確認:
```bash
psql "$PADDOCK_DB_URL" -c "SELECT race_id, venue, race_num, distance, surface FROM race_cards ORDER BY race_num;"
psql "$PADDOCK_DB_URL" -c "SELECT gate_num, horse_num, horse_name, jockey FROM horse_entries WHERE race_id='2026-3-nakayama-8-1R' ORDER BY horse_num;"
```

### netkeiba から取得する

JRA 成績 PDF が未公開の当日・直近レースを予想するためのデータ源。出馬表・オッズ・近走・確定結果を
netkeiba から取得して DB に入れる。netkeiba は第三者の公開サーバなので、リクエスト間隔（`--interval`）で
礼節に留意する。

#### 出馬表＋オッズ＋近走（fetch-card）

当日の出馬表とオッズ（単複・馬連・馬単・三連複・三連単）を取得し、`race_cards` / `horse_entries` /
`race_odds` に保存する。各馬の過去走（`horse_past_runs`）も既定で取り込む。これを実行しておくと
`predict`（対話セッション）で買い目推奨まで出せる（`analyze predict` は確率表のみ）。

netkeiba 12 桁は `YYYY` + 場コード(2) + 回(2) + 日(2) + R(2)。以下はいずれも `2026-3-nakayama-8-1R`
（= netkeiba `202606030801`、中山=場コード 06）の同一レースを 2 通りの入力で指定する例:

```bash
# netkeiba の race_id（12 桁）を直接指定
cargo run -p fetch-card -- 202606030801

# 構成要素から組み立て（全指定が必要・上と同じレース）
cargo run -p fetch-card -- --year 2026 --venue nakayama --round 3 --day 8 --race 1

cargo run -p fetch-card -- 202606030801 --force          # 取得済みカードを上書き
cargo run -p fetch-card -- 202606030801 --skip-history   # 過去走の取り込みを省略
cargo run -p fetch-card -- 202606030801 --interval 1500  # リクエスト間隔(ms)
```

#### 出走馬の近走（fetch-history）

出馬表から各馬の `horse_id` を引いて近走を取得し、`results` に取り込む（前走フォーム等の予想シグナル用）。

```bash
cargo run -p fetch-history -- 202606030801 202606030802   # race_id（複数可・中山3回8日 1R/2R）
cargo run -p fetch-history -- --horse-id 2021104500       # 出馬表をバイパスして horse_id 直指定
cargo run -p fetch-history -- 202606030801 --no-backfill  # pdf 成績行への horse_id backfill を抑止
```

#### 確定結果で results を再取込（fetch-results）

netkeiba 結果ページから既存 `results` を再取得し、`jockey` / `trainer` を netkeiba の略名表記に揃える。
PDF 由来の馬主名混入や調教師名の不一致を解消し、predict の entry↔results join を噛み合わせる
（`races` 行は更新しない）。

```bash
cargo run -p fetch-results -- --from 2026-01-01 --to 2026-03-31
cargo run -p fetch-results -- --from 2026-06-01 --interval 1500
```

#### 単複オッズの時系列収集（odds-collect）

指定日の全レースを間隔スイープし、**未発走レースの単複オッズだけ**を再取得して `race_odds_snapshots` に
append する時系列コレクタ（モデル非依存）。EV・買い目は一切計算しない（確率と収集の分離）。発走済みの
レースは順次対象から外れ、全レース発走で自動終了する。前提: 当日 `fetch-card` 済み（発走時刻が要る）。

```bash
cargo run -p odds-collect -- --date 2026-06-01           # 既定: 間隔 15 分 / リクエスト間 2000ms
cargo run -p odds-collect -- --date 2026-06-01 --once    # 1 スイープのみ（cron 等の定期起動向け）
```

- `--interval <分>`: スイープ間隔（最小 1 分。既定 15）。
- `--scrape-delay <ms>`: 1 リクエストごとの待機（netkeiba への礼節。既定 2000）。

## 集計・予想する

### 集計コマンド（analyze）

馬の傾向（名前の一部・表記ゆれでも引ける。複数該当時は候補一覧を提示）:
```bash
cargo run -p analyze -- horse "イクイノックス"
cargo run -p analyze -- horse "ダイワ"        # 中間一致 → 複数候補なら一覧表示
```

コースの枠順傾向:
```bash
cargo run -p analyze -- course 中山 2000 turf      # 芝
cargo run -p analyze -- course 阪神 1200 dirt      # ダート
```

騎手の傾向:
```bash
cargo run -p analyze -- jockey "ルメール"
```

調教師の傾向:
```bash
cargo run -p analyze -- trainer "藤沢"
```

枠順は次のように 3 グループに集約:
- Inner: 1〜3 枠
- Middle: 4〜6 枠
- Outer: 7〜8 枠

### 1 レースの確率推定（analyze predict）

出馬表が取り込み済みの 1 レースについて、各馬の win/place/show 確率を推定して表示する（DB 変更なし）。
`win ≤ place ≤ show` の単調性を保証し、place/show は出走全体で 2.0 / 3.0 に正規化した上で単調化する。

```bash
cargo run -p analyze -- predict 2026-3-nakayama-8-1R
cargo run -p analyze -- predict 2026-3-nakayama-8-1R --blend-alpha 0.7        # 市場オッズ(単勝)をブレンド
cargo run -p analyze -- predict 2026-3-nakayama-8-1R --track-condition 稍重    # 当日の馬場を factor に加える
```

race_id は DB に保存される paddock 形式（`{年}-{回}-{場slug}-{日}-{R}R`、例 `2026-3-nakayama-8-1R`）で渡す。

- `--blend-alpha <α>`: モデル確率 α と市場 implied 確率 (1-α) の線形ブレンド。implied 確率は最新オッズスナップショット（時刻制約なし）から取る。未指定はモデルのみ。
- `--track-condition <良/稍重/重/不良>`: 出馬表 PDF に馬場状態は無いため手で渡す（`稍` `不` の略記可）。

シグナルは「馬の芝ダ／距離帯／馬場別成績・騎手／調教師の芝ダ成績・コース×枠・前走フォーム・
斤量のレース内相対」を重み付き平均し、少データ馬はベイズ縮約で母集団 prior へ寄せる。

### 予想セッション（predict・対話）

1 日分のレースを順番に処理し、買い目推奨を確認しながら賭け金と払い戻しを記録する対話型 CLI。

```bash
# 新規開始（--budget 必須）
cargo run -p predict -- --date 2026-06-01 --budget 10000

# 中断したセッションを保存済みの残高から再開
cargo run -p predict -- --date 2026-06-01 --resume

# 収支サマリと買い目明細を表示（読み取り専用）
cargo run -p predict -- --date 2026-06-01 --summary

# レース確定後、netkeiba の確定払戻で購入済み買い目を自動精算
cargo run -p predict -- --date 2026-06-01 --settle
```

- 各レースで確率表と買い目推奨（期待値ベース）を表示する。
- `[y=推奨通り / e=編集 / s=スキップ]` で購入方法を選び、レース後に**買い目ごとに**実際の払い戻し額を入力する（命中精度・回収率の分析に使う）。
- 推奨額は券種ごとに固定予算を割り当て、券種内は各点へ 100 円単位で均等配分する（予算が賄える範囲で薄い相手にも同額が乗り取りこぼさない、賄えない端数の点は買わない）。券種予算ちょうどに収める。
- `--settle`: 確定後に netkeiba の払戻で購入済み買い目の payout を自動セットし、収支・回収率を更新する（冪等。未確定レースはスキップ）。手入力の代わりに使える。
- オッズ取得済み（`fetch-card` で `race_odds` 整備済み）のレースは買い目推奨が出る。未取得のレースはスキップのみ受け付ける。
- 出馬表（`race_cards`）が取り込み済みであることが前提（「データを取り込む」参照）。

#### セッションの永続化（`--resume` / `--summary`）

- セッションは **1 開催日 = 1 セッション**として `predict_sessions` に、購入した買い目は `predict_bets` に保存され、レース確定ごとに 1 トランザクションで更新する。
- `--resume`: 同日の未完了セッションを保存済み残高から再開し、購入済みレースはスキップする（スキップしただけのレースは再提示される）。完了済みセッションでは `--summary` を案内する。
- `--summary`: 開始予算・残高・総投資・総払戻・収支・回収率と買い目明細を表示する（DB を変更しない）。
- 既にセッションがある日に `--resume` なしで実行すると、二重作成を避けるため中止して `--resume` / `--summary` を案内する。

> オッズは `fetch-card`（netkeiba）が `race_odds` テーブルに永続化する。当日のカードを `fetch-card` で
> 取り込んでおけば買い目推奨まで出る。オッズ未取得のレースはスキップのみになる。

### 発走直前の EV/ROI 監視（predict-watch）

朝の +EV は発走直前に剥がれるため、EV/ROI 判定は発走直前のフレッシュなオッズで行う。predict-watch は
指定開催日の発走前レースを定期スキャンし、オッズを再取得して ROI を再計算、ゲート以上のレースを買い目付きで
通知する **decision-support** ツール（ADR 0055/0060。張る/見送り/増額の最終判断は人間が行い、軸は動かさない）。
predict のセッション記録（`predict_sessions` / `predict_bets`）には書き込まない（オッズスナップショット
`race_odds` は再取得・保存する）。全レース発走で自動終了する。

```bash
cargo run -p predict-watch -- --date 2026-06-01           # 既定: 窓 40 分 / 間隔 5 分 / ROI ゲート 1.0 / α=0.2
cargo run -p predict-watch -- --date 2026-06-01 --once    # 1 スイープのみ（テスト・cron 用）
cargo run -p predict-watch -- --date 2026-06-01 \
  --race-budget-override 2026-3-hakodate-2-6R=7000        # 特定レースだけ予算を上書き（増額の執行入力）
```

- `--window <分>`: 発走まで残りこの時間以内のレースだけオッズを再取得する先読み窓（既定 40）。
- `--interval <分>`: スイープ間隔（既定 5）。
- `--roi-gate <倍率>`: 🔶 買い妙味として目立たせる ROI 閾値（既定 1.0 = 100%）。
- `--notify-gate <倍率>`: 🔍 検証候補として表示に残す通知閾値。未指定なら `min(roi_gate, 0.7)`。明示指定は roi_gate 以下であること。
- `--race-budget <円>`: 買い目（軸ながしポートフォリオ）組成の 1 レース予算（既定 5000）。`--race-budget-override <race_id>=<円>` で per-race 上書き（複数はフラグを繰り返す）。
- `--blend-alpha <α>`: 市場単勝ブレンドのモデル重み（未指定は本番既定 α=0.2）。
- `--scrape-delay <ms>`: オッズ再取得のリクエスト間待機（既定 3000）。

### 買い目シミュレーション（simulate）

買い目ポートフォリオの収支シミュレータ。全着順を列挙して払戻・収支を集計する。

```bash
cargo run -p simulate -- --input bets.json            # 買い目定義 JSON
cargo run -p simulate -- --input bets.json --main 5-1-3   # 本線の着順を上書き
echo '{ ... }' | cargo run -p simulate                   # 標準入力からも可
```

## 検証する（analyze backtest）

過去の確定レースに対し、その時点までの統計だけ（walk-forward、リークなし）で予想を再現し、
的中率・想定回収率・Brier・LogLoss を集計する。重みやパラメータの良し悪しを測るためのもの。

```bash
cargo run -p analyze -- backtest --from 2026-01-01 --to 2026-03-31
cargo run -p analyze -- backtest --from 2026-01-01 --to 2026-03-31 --blend-alpha 0.7
cargo run -p analyze -- backtest --from 2026-01-01 --to 2026-03-31 --shrinkage-m 10 --recency-half-life 60
```

- `--blend-alpha <α>`: 当時オッズの implied 確率とブレンドして評価。
- `--shrinkage-m <m>`: ベイズ縮約の擬似カウント。5/10/20/50 等のスイープで校正改善を比較する。
- `--recency-half-life <days>`: 直近成績を重く見る時間減衰の半減期。30/60/90 等で比較。

回収率はオッズ取得済みレース（`race_odds`）が母数。スコア挙動を変える改善はこの backtest を通してから採用する。

## 予想を保存・閲覧する

### 予想を DB に保存する（ingest-predictions）

予想（印・短評・買い目・結果）を構造化レコードとして DB に永続化する。**DB が正で、pad の MD は
このレコードから生成する**（手書き MD は不要）。予想を作るときは JSON（仕様: `docs/specifications/prediction-json.md`）を
吐いて取り込み、必要なら `--render` で MD を生成してビューア（web-viewer）で見る。

```bash
# 取り込み（stdin もしくは --input <file>）
cat pred.json | cargo run -p ingest-predictions
cargo run -p ingest-predictions -- --input pred.json

# パース・検証のみ（保存しない）
cargo run -p ingest-predictions -- --input pred.json --dry-run

# DB の全予想を pad の MD に生成（PADDOCK_PAD_DIR / --pad-dir で出力先指定）
cargo run -p ingest-predictions -- --render
```

- レースは `(date, venue, race_num)` で一意（同キーの再取り込みは upsert＝冪等）。`race_id` は
  `races`/`race_cards` に一致があれば自動解決して保持する。
- 保存先は `PADDOCK_DB_URL`（Postgres）。

### 予想 MD をブラウザで見る（web-viewer）

Obsidian vault に書き出した予想 Markdown（`pad/{YYYYMMDD}/{開催場}/{RR}R.md`）を、ローカル Web サーバでブラウザ表示する軽量ビューア。確率表・印・買い目テーブルを読みやすく整形し、左ツリー（日付 > 開催場 > レース）から選んで右ペインに表示する。DB には触れず pad ディレクトリを読むだけ。

```bash
# 起動（http://localhost:8787 を開く）
cargo run -p web-viewer

# pad ディレクトリ／ポートを変える場合
PADDOCK_PAD_DIR="/path/to/vault/pad" PAD_WEB_PORT=9000 cargo run -p web-viewer
```

- `PADDOCK_PAD_DIR`: 予想 MD のルート（既定は iCloud Obsidian vault の `pad/`）。
- `PAD_WEB_PORT`: 待ち受けポート（既定 `8787`）。
- 表示は永続化済みの MD をそのままレンダリングするだけで、自動更新やセッション操作はしない。

### REST API とライブ盤面（api-server + web）

DB の集計・予想・セッションを HTTP で提供する REST API（actix-web、read 中心）と、それを表示する
React SPA。盤面（`/races/{race_id}/board`）で確率・オッズ・印・買い目をブラウザで一覧できる。

```bash
# API サーバ（既定 127.0.0.1:8080。PADDOCK_SERVER_ADDR で変更）
cargo run -p api-server

# Web SPA（Vite dev server。http://localhost:5173 を開く）
cd web && npm install && npm run dev
```

- API は `/api/*`（races / board / predictions / analyze / live / sessions 等）。`/docs` に Swagger UI、
  `/api-docs/openapi.json` に OpenAPI ドキュメントを配信する。
- OpenAPI は utoipa のコードファーストで、スナップショットを `docs/api/openapi.json` に保持する。
  web の TypeScript 型はここから `npm run gen:api` で生成する。
- dev では Vite が `/api` を API サーバへ proxy する（同一オリジン化で CORS 不要）。proxy 先は
  `PADDOCK_API_TARGET` で変更できる（既定 `http://localhost:8080`）。
- compose では `api`（:8080）と `web`（nginx、:8081）を常駐サービスとして起動できる:
  `docker compose -f deployments/compose.yaml up -d postgres api web`

## 運用

### 取り込みをコンテナで隔離実行（importer）

成績取り込み（`parse-pdf fetch`）は OCR(tesseract) が CPU を食い、年単位のバックフィルで数時間かかる。
取り込み中も開発機を軽く保つため、importer を**コンテナで CPU キャップ付き隔離実行**できる。DB は
compose の `postgres` サービスへ接続するため bind mount は使わない。

```bash
# 1) イメージをビルド（mutool + tesseract(jpn) 同梱、paddock-parse-pdf を release ビルド）
docker compose -f deployments/compose.yaml build importer

# 2) detach（run-and-forget）で取り込み。例: 2025 年の全開催
docker compose -f deployments/compose.yaml run --rm -d importer fetch --year 2025

# 3) 進捗ログ（コンテナ ID は run の出力 or `docker ps`）
docker logs -f <container-id>
```

- **礼節ペーシング**: `fetch` は entrypoint が `-j 1 --interval 3 --max-rps 0.3` を既定で補う
  （ノーペーシングだと JRA に IP ブロックされうるため。明示指定した値は尊重する）。
- **CPU キャップ**: `deploy.resources.limits.cpus: "2.0"`。compose のバージョン差で効かない場合は
  `docker compose ... run --rm -d --cpus 2 importer fetch ...` で明示できる。`docker stats` で使用量を確認できる。
- **中断・再開（冪等）**: 取得状態は DB の `fetch_history` に記録される。中断後に同じコマンドを再実行すると
  取得済み開催はスキップされる（`--force` で再取得）。
- **DB 共有**: importer は常に共有の `paddock` データベース（`postgres` サービス）に書き込む。
  ホスト側 dev は読み取り中心で並行作業できる。

### worktree ごとの DB 分離

各 worktree は 1 つの PG サーバを共有し、**database 名を変えて分離する**（`.env` の `PADDOCK_DB_URL`
末尾の DB 名を worktree 別にする。例: `.../paddock_feat_xxx`）。別 database は `seed-db.sh` が作成・複製する。

### seed / reset（並走 worktree）

新しい worktree の database は空なので、実データで predict / backtest / analyze を回すには golden
（ingest 済みの DB。既定: 同サーバの `paddock`）から複製する。`psql` / `pg_dump`（libpq クライアント）が要る
（`pg_dump` のメジャー版はサーバ（PG 17）以上が必要。例: `brew install postgresql@17`）。

```bash
scripts/seed-db.sh                       # golden(paddock) → $PADDOCK_DB_URL へ複製
scripts/seed-db.sh --from <golden_url>   # golden を明示
scripts/seed-db.sh --to <target_url>     # 配置先 DB を明示
PADDOCK_GOLDEN_DB_URL=<url> scripts/seed-db.sh

scripts/reset-db.sh                      # $PADDOCK_DB_URL の database を空に戻す
scripts/reset-db.sh --to <target_url>    # 対象 DB を明示
```

- seed は配置先 database を作り直し、golden を `pg_dump | psql` で丸ごと複製する
  （`_sqlx_migrations` も含むので配置後に再マイグレートは走らない）。
- reset は対象 database を `DROP/CREATE` して空にする。次回アプリ起動で自動マイグレートされる。
- seed / reset とも、対象 database を使用中のアプリは停止してから実行する。
- golden（`paddock`）への reset は誤爆防止で既定中断する。意図的なら `--force`。
- DROP/CREATE DATABASE の管理接続には同サーバの `postgres` database を使う（compose の PG には存在）。

### バックアップ・補助スクリプト

- `scripts/backup-db.sh`: DB 全体を custom-format dump でタイムスタンプ付き退避＋世代管理する
  （`race_odds_snapshots` 等の再取得不能な蓄積資産を volume 喪失から守る）。復元手順は
  `deployments/db/BACKUP.md`、日次実行などの launchd ジョブは `deployments/launchd/` を参照。
- `scripts/check-adr-numbers.sh`: ADR 番号（`docs/adr/NNNN-*.md`）の重複を機械検出する（CI / pre-push 用）。
- `scripts/harness/`: 学習型モデル評価ハーネス（backtest の忠実性ゲート等。`scripts/harness/README.md` 参照）。

## 開発

ワークスペース全体ビルド:
```bash
cargo build --workspace
```

テスト（同梱サンプル PDF を使った統合テストを含む）:
```bash
cargo test
```

整形・lint:
```bash
cargo fmt
cargo clippy --all-targets
```

ドキュメント:

- `docs/adr/` … アーキテクチャ・ルール変更の決定記録（ADR。棄却した案も記録する）
- `docs/specifications/` … 確率推定・backtest・買い目選定・予想 JSON などの仕様書
- `docs/api/openapi.json` … REST API の OpenAPI スナップショット（utoipa コードファースト。web の型生成の入力）

## 予想ワークフロー全体像

```
fetch-card         … 出馬表＋オッズ＋近走を取得（race_cards / race_odds / horse_past_runs）
  ├─ (fetch-history で近走を results に補強)
  └─ (odds-collect で単複オッズの時系列を終日収集・任意)
analyze predict    … 1 レースの確率を確認（単発・DB 変更なし）
predict            … 1 日分を対話的に予想・購入記録（--budget で開始）
  ├─ ingest-predictions … 予想（印・買い目）を DB に保存 → web-viewer / api-server + web で閲覧
  ├─ predict-watch … 発走直前のオッズで EV/ROI を再計算して通知（セッション記録は不変）
  └─ レース確定後 → fetch-results で結果を取り込み、predict --settle で自動精算
analyze backtest   … 過去レンジで予想ロジックを walk-forward 検証
```

「結果を見ずに 1R から予想して後で答え合わせする」再実行可能なハーネスは
`scripts/predict-check/README.md` を参照。

## アーキテクチャ

クリーンアーキテクチャに準拠した workspace 構成。

```
依存方向: apps → interface → use-case → domain
                                         ↑
                                infrastructure (config)
```

```
src/
├── domain/                   コアエンティティ＋値オブジェクト
├── use-case/                 Repository / PdfParser / EntryParser 等のトレイトと Interactor
├── interface/
│   ├── pdf-parser/           mutool + OCR ハイブリッドで PDF→Race（成績）
│   ├── pdf-ocr/              tesseract サブプロセスで PDF→OCR 行
│   ├── entry-parser/         mutool stext.json で PDF→RaceCard（出馬表）
│   ├── jra-fetcher/          JRA PDF の共有 HTTP フェッチャ（タイムアウト・リトライ・403/404 判定）
│   ├── netkeiba-scraper/     netkeiba の出馬表・オッズ・近走・払戻のスクレイパ
│   ├── scraper-util/         スクレイパ共通ユーティリティ（EUC-JP デコード等）
│   ├── predict-format/       予想確率・根拠の CLI 表示整形（predict / predict-watch 共用）
│   ├── rest-controller/      REST API のハンドラ・ルータ・OpenAPI 定義（utoipa）
│   └── rdb-gateway/          sqlx で Repository 実装（Postgres）
├── infrastructure/
│   └── config/               環境変数から Config を構築
└── apps/
    ├── parse-pdf/            CLI: 成績 PDF 取り込み（ローカル / URL / fetch）
    ├── parse-entries/        CLI: 出馬表 PDF 取り込み
    ├── analyze/              CLI: horse / course / jockey / trainer / predict / backtest
    ├── predict/              CLI: 予想セッション（対話・収支記録）
    ├── predict-watch/        CLI: 発走直前の EV/ROI 監視・通知
    ├── odds-collect/         CLI: 単複オッズの時系列収集
    ├── fetch-card/           CLI: netkeiba 出馬表＋オッズ＋近走を取得
    ├── fetch-history/        CLI: netkeiba 近走を results に取得
    ├── fetch-results/        CLI: netkeiba 確定結果で results を再取込
    ├── ingest-predictions/   CLI: 予想 JSON の DB 保存・pad MD 生成
    ├── simulate/             CLI: 買い目ポートフォリオの収支シミュレータ
    ├── api-server/           REST API サーバ（actix-web + utoipa）
    └── web-viewer/           pad MD のブラウザビューア

web/                          React SPA（Vite。api-server の /api を表示するライブ盤面）
```

## 既知の制約

- 斤量・人気・調教師・騎手は OCR 非依存で決定的に取得する。
  - 斤量は `mutool -F stext.json` の座標索引から確定する。
  - 人気は単勝オッズの昇順順位から算出する（同オッズは同順位）。
  - 調教師・騎手は stext 座標索引から列単位で抽出する（テキスト抽出だと隣の馬主名や斤量が当該列に連結するため）。
  - 着順は mutool の行順（完走順）が土台で、OCR 結果が完全集合として信頼できる場合のみ上書きする。
  - **上り3F** は未対応（`HorseResult` に該当フィールドが無い）。
- 競走除外・出走取消・競走中止の馬（完走しなかった馬）は `finishing_position = NULL` で保存される。
- タイム・オッズは mutool テキスト抽出ベースで取得（同サンプル PDF で取得率は概ね 8〜9 割超）。同タイム馬（`〃` 表記）はタイムが取れない。
- 騎手名・馬名は取り込み・検索の双方で共通の正規化（全角英字/数字→半角、全角ピリオド `．`→`.`、半角カナ→全角）が適用され、`Ｃ．ルメール` でも `C.ルメール` でも、`ｲｸｲﾉｯｸｽ` でも `イクイノックス` でも引ける。
- 馬名・騎手名は**部分一致（中間一致）**で検索でき、複数候補は一覧提示する（`analyze` の `horse` / `jockey`）。既存 DB に未正規化の値が残る場合は対象 PDF を再 ingest すると揃う。

## ライセンス

MIT — `LICENSE` 参照。
