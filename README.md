# paddock

JRA 公式のレース成績 PDF をパースして SQLite に蓄積し、その実績から各馬の勝率・連対率・複勝率を
推定して期待値・Kelly で買い目まで出す競馬予想 CLI 群（Rust 製）。「成績を貯める」基盤に加え、
「貯めた成績で予想する」レイヤ（確率推定・買い目推奨・セッション収支記録・backtest 検証）を持つ。
当日の出馬表・オッズ・近走・確定結果は netkeiba からも取得できる。

## 何ができるか

**集計（analyze）**

- 馬の特性集計: 芝/ダート、距離帯、枠順（内/中/外）、馬場状態ごとの勝率・連対率
- コースの特性集計: 競馬場 × 距離 × 芝ダ で、枠順別の勝率・連対率
- 騎手・調教師の基本スタッツ: 全体／芝ダ／枠順別の勝率・連対率

**予想（analyze predict / predict）**

- 1 レースの win/place/show 確率推定（馬・騎手・調教師・コース・馬場・前走フォーム・斤量などの
  シグナルを重み付き平均し、ベイズ縮約・市場オッズブレンドで補正）— `analyze predict` / `predict` 共通
- 期待値・Kelly 配分にもとづく買い目推奨（単複・馬連・馬単・三連複・三連単）— `predict` のみ
- 1 開催日を対話的に予想して賭け金・払い戻し・収支を記録するセッション — `predict`
- 日付範囲での backtest 検証（的中率・回収率・Brier・LogLoss）— `analyze backtest`

**取得（parse-pdf / parse-entries / fetch-\*）**

- JRA 成績 PDF・出馬表 PDF の取り込み（ローカル / URL / 開催指定の自動取得）
- netkeiba からの当日出馬表・オッズ・近走・確定結果の取得

## 必要環境

- macOS (Apple Silicon / Intel)。Linux でも動作見込み。
- Rust ツールチェイン: `rust-toolchain.toml` で `1.93.0` 固定
- **mupdf-tools** (`mutool` バイナリ): PDF テキスト抽出に使用
  ```bash
  brew install mupdf-tools
  ```
  JRA の PDF は埋め込みフォントを使うため、純 Rust の `pdf-extract` では文字化けする。`mutool` をサブプロセス経由で呼び出して回避している。
- **tesseract** + **tesseract-lang**（jpn パック）: 成績 PDF の **着順の検証 override** と、mutool が
  座標索引で取りこぼした行の補完に使う画像 OCR。`parse-pdf` 起動時に preflight チェックが走り、
  未インストール／jpn パック未導入ならその場でエラー終了する。
  ```bash
  brew install tesseract tesseract-lang   # 日本語パック含む
  tesseract --list-langs                  # jpn が表示されることを確認
  ```
  斤量・人気・調教師・騎手はもう OCR ではなく mutool の CID テキスト／stext 座標／単勝オッズ順位から
  決定的に取得する（斤量・人気は #124 / ADR 0018、調教師・騎手は従来からの stext 抽出）。OCR 経路は
  これらに対しては冗長だが、着順は依然 OCR 由来の値で上書きする余地があるため tesseract は必須のまま
  （撤去は別 PR で検討）。

## ビルド

```bash
cargo build --release
```

## PDF を取り込む

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

### 開催指定で JRA から自動取得（fetch）

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

- 既定の並列度は **CPU コア数**。`-j 1` を指定すると従来どおりの**逐次・404/403 境界探索**になり、`--interval`（既定 1 秒のリクエスト間ウェイト）を尊重する。
- `-j 2` 以上では候補グリッド（場 × 回 × 日）を列挙して並列に取得する（境界での早期打ち切りはせず、非存在は 404/403 として集計）。リクエスト間ウェイトは無し。
- **総アクセス規模**: 並列パスはグリッドを全列挙するため、`fetch --year YYYY`（全場・全回・全日）では最大で `場数 × 8(回上限) × 14(日上限)` ≒ **1,000 件超**の GET を JRA に発行しうる（未公開・非存在は 404/403 で軽く弾かれる）。同時実行（in-flight）数は **CPU コア数**で上限管理され、後段 OCR が律速になるため実効リクエストレートは穏当だが、JRA は第三者の公開サーバなので礼節に留意する。
- **`--max-rps`（任意のレート上限）**: JRA への秒間リクエスト数に上限を設けられる。全 fetch（逐次・並列とも）で共有するグローバルな下限間隔として効き、実際にネットワーク GET する取得だけを間引く（`fetch_history` ヒットのスキップは対象外なので再実行は遅くならない）。主用途は並列パスのピークレート抑制。`-j 1` でも作用し、その場合は `--interval` と併用され大きい方のウェイトが支配的になる。未指定なら無制限（既定）。
  ```bash
  cargo run --release -p parse-pdf -- fetch --year 2025 -j 8 --max-rps 2   # 8 並列・JRA へは最大 ~2 req/s
  ```
- 礼節を最優先するなら `-j 1`（逐次・1 秒間隔・境界での早期打ち切り）に戻せば総アクセスも最小化できる。
- 各開催の OCR は `tesseract` を起動する。並列時は過剰なスレッド競合を避けるため OCR を 1 プロセス 1 スレッドに固定する（`OMP_THREAD_LIMIT=1` / `OMP_NUM_THREADS=1` を自動設定）。
- `fetch_history` への記録・スキップ・`--force` の挙動は逐次と同じ。

### 抽出ロジック

抽出は常に **mutool テキスト/座標抽出 + OCR 補完** のハイブリッド方式で動作する（モード切替なし）。

1. 起動時に `tesseract` バイナリと jpn 言語パックの存在を preflight チェックし、欠けていれば即エラー終了する
2. `mutool draw -F text` で PDF テキストを抽出し、土台となる Race / 結果テーブルを構築（着順は完走の行順を土台に置く。最終確定はステップ 4 を参照）
3. `mutool draw -F stext.json` の座標索引から **騎手・調教師・斤量** を確定する。斤量は CID 数字で読めるため
   EdiF 復号は不要（#124 / ADR 0018）。**人気は単勝オッズの昇順順位**から決定的に算出する（EdiF 非依存）
4. PDF を PNG 化して OCR をかけ、着順は OCR 抽出結果が「1〜頭数の完全集合の半分以上を占める」場合のみ
   上書き採用し、そうでなければ mutool の行順 fallback を使う。斤量・調教師（座標索引が取りこぼした行）と
   人気（オッズ欠落で順位が付かなかった行）の OCR 補完も残っているが、いずれもステップ 2〜3 で値が埋まらな
   かった行にのみ効くため #124 以降は基本冗長

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
sqlite3 data/paddock.db "SELECT race_id, race_num, surface, distance FROM races ORDER BY race_id;"
```

## 集計コマンド

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

### 1 レースの確率推定（`analyze predict`）

出馬表が取り込み済みの 1 レースについて、各馬の win/place/show 確率を推定して表示する（DB 変更なし）。
`win ≤ place ≤ show` の単調性を保証し、place/show は出走全体で 2.0 / 3.0 に正規化した上で単調化する（ADR 0007）。

```bash
cargo run -p analyze -- predict 2026-3-nakayama-8-1R
cargo run -p analyze -- predict 2026-3-nakayama-8-1R --blend-alpha 0.7        # 市場オッズ(単勝)をブレンド（#72）
cargo run -p analyze -- predict 2026-3-nakayama-8-1R --track-condition 稍重    # 当日の馬場を factor に加える（#73）
```

race_id は DB に保存される paddock 形式（`{年}-{回}-{場slug}-{日}-{R}R`、例 `2026-3-nakayama-8-1R`）で渡す。

- `--blend-alpha <α>`: モデル確率 α と市場 implied 確率 (1-α) の線形ブレンド。implied 確率は最新オッズスナップショット（時刻制約なし）から取る。未指定はモデルのみ。
- `--track-condition <良/稍重/重/不良>`: 出馬表 PDF に馬場状態は無いため手で渡す（`稍` `不` の略記可）。

シグナルは「馬の芝ダ／距離帯／馬場別成績・騎手／調教師の芝ダ成績・コース×枠・前走フォーム（#31/#76）・
斤量のレース内相対（#135）」を重み付き平均し、少データ馬はベイズ縮約（#75）で母集団 prior へ寄せる。

### backtest（予想ロジックの検証）

過去の確定レースに対し、その時点までの統計だけ（walk-forward、リークなし）で予想を再現し、
的中率・想定回収率・Brier・LogLoss を集計する。重みやパラメータの良し悪しを測るためのもの。

```bash
cargo run -p analyze -- backtest --from 2026-01-01 --to 2026-03-31
cargo run -p analyze -- backtest --from 2026-01-01 --to 2026-03-31 --blend-alpha 0.7
cargo run -p analyze -- backtest --from 2026-01-01 --to 2026-03-31 --shrinkage-m 10 --recency-half-life 60
```

- `--blend-alpha <α>`: 当時オッズの implied 確率とブレンドして評価（#72）。
- `--shrinkage-m <m>`: ベイズ縮約の擬似カウント（#75）。5/10/20/50 等のスイープで校正改善を比較する。
- `--recency-half-life <days>`: 直近成績を重く見る時間減衰の半減期（#75 Phase B）。30/60/90 等で比較。

`scripts/predict-check/README.md` が言及する「144R backtest で検証」の実体はこのコマンド。回収率は
オッズ取得済みレース（`race_odds`）が母数。スコア挙動を変える改善はこの backtest を通してから採用する。

## 予想セッション（対話）

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

- 各レースで確率表と買い目推奨（期待値・Kelly 配分）を表示する。
- `[y=推奨通り / e=編集 / s=スキップ]` で購入方法を選び、レース後に**買い目ごとに**実際の払い戻し額を入力する（命中精度・回収率の分析に使う）。
- 推奨額は Kelly 配分を比例縮小方式で算出し、合計が残高を超えないよう収める。
- `--settle`: 確定後に netkeiba の払戻で購入済み買い目の payout を自動セットし、収支・回収率を更新する（冪等。未確定レースはスキップ）。手入力の代わりに使える。
- オッズ取得済み（`fetch-card` で `race_odds` 整備済み）のレースは買い目推奨が出る。未取得のレースはスキップのみ受け付ける。
- 出馬表（`race_cards`）が取り込み済みであることが前提（「出馬表を取り込む」「netkeiba から取得する」を参照）。

### セッションの永続化（`--resume` / `--summary`）

- セッションは **1 開催日 = 1 セッション**として `predict_sessions` に、購入した買い目は `predict_bets` に保存され、レース確定ごとに 1 トランザクションで更新する。
- `--resume`: 同日の未完了セッションを保存済み残高から再開し、購入済みレースはスキップする（スキップしただけのレースは再提示される）。完了済みセッションでは `--summary` を案内する。
- `--summary`: 開始予算・残高・総投資・総払戻・収支・回収率と買い目明細を表示する（DB を変更しない）。
- 既にセッションがある日に `--resume` なしで実行すると、二重作成を避けるため中止して `--resume` / `--summary` を案内する。

> オッズは `fetch-card`（netkeiba）が `race_odds` テーブルに永続化する。当日のカードを `fetch-card` で
> 取り込んでおけば買い目推奨まで出る。オッズ未取得のレースはスキップのみになる。

## 予想をブラウザで見る（web-viewer）

Obsidian vault に書き出した予想 Markdown（`pad/{YYYYMMDD}/{開催場}/{RR}R.md`）を、ローカル Web サーバでブラウザ表示する軽量ビューア。確率表・印・買い目テーブルを読みやすく整形し、左ツリー（日付 > 開催場 > レース）から選んで右ペインに表示する。DB には触れず pad ディレクトリを読むだけ。

```bash
# 起動（http://localhost:8787 を開く）
cargo run -p web-viewer

# pad ディレクトリ／ポートを変える場合
PADDOCK_PAD_DIR="/path/to/vault/pad" PAD_WEB_PORT=9000 cargo run -p web-viewer
```

- `PADDOCK_PAD_DIR`: 予想 MD のルート（既定は iCloud Obsidian vault の `pad/`）。
- `PAD_WEB_PORT`: 待ち受けポート（既定 `8787`）。
- 表示は永続化済みの MD をそのままレンダリングするだけで、自動更新やセッション操作はしない（フル GUI 化は #34）。

## 予想を DB に保存する（ingest-predictions）

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
- 保存先は `PADDOCK_DB_URL`（既定 `data/paddock.db`）。検索・絞り込み・的中率の集計は別途（#145）。

## DB

- 既定パス: `data/paddock.db`
- スキーマは初回起動時に自動マイグレート（`deployments/db/migrations/`）
- 環境変数 `PADDOCK_DB_URL` で接続先を上書き可能（例: `sqlite://./other.db?mode=rwc`）
- DB を作り直したい場合は `data/paddock.db` を消してから取り込み直し
- 接続プールは WAL・外部キー有効に加え `busy_timeout=5s` を設定済み（同一クローン内で predict と analyze を
  並行起動したときのロック即時失敗を緩和する）。これは **プロセス間の SQLite ファイルロックの再試行待ち**で、
  プール（`max_connections`）の接続待ちタイムアウトとは別レイヤ。

### 並走クローンの seed / reset

並走クローン（worktree / 独立 clone）は DB を共有しない（`PADDOCK_DB_URL` 既定は相対パスで各 cwd 配下）。
新しいクローンは空の DB から始まるため、predict / backtest / analyze を実データで回すにはフル re-ingest が要る。
これを避けるため、ingest 済みの clone（golden）から DB スナップショットを配置する `scripts/` を用意している。
（`sqlite3` CLI が必要。既定パスは cwd 相対なので **対象クローンの root で実行**する。）

```bash
# 並走クローンを切る → そのクローン内で seed → 実データで予想/解析
scripts/seed-db.sh            # primary clone を git 自動検出し、その data/paddock.db を ./data へ配置
scripts/seed-db.sh --from /path/to/golden.db   # golden を明示
scripts/seed-db.sh --to /path/to/data          # 配置先 data ディレクトリを明示
PADDOCK_GOLDEN_DB=/path/to/golden.db scripts/seed-db.sh

scripts/reset-db.sh           # ./data/paddock.db を .bak へ退避して空に戻す（再 seed / 再 ingest 前提）
scripts/reset-db.sh --to /path/to/data   # 対象 data ディレクトリを明示
scripts/reset-db.sh --no-backup          # 退避せず削除
```

- seed は `sqlite3` の `.backup`（オンラインバックアップ）で一貫スナップショットを作るため、golden が
  **実行中でも安全**で、コミット済み状態と WAL を取り込んだ単一ファイルを配置する（WAL/SHM 残骸を残さない）。
- 既定の golden 元は `git rev-parse --git-common-dir` から辿った primary clone の `data/paddock.db`。
  worktree 以外の独立 clone から seed する場合は `--from` か `PADDOCK_GOLDEN_DB` で明示する。
- 配置前に既存 `data/paddock.db`（と `-wal`/`-shm`）は `.bak-<日時>` に退避される（`data/*.bak-*` は gitignore 済み）。
  退避を戻すときは同じ `<日時>` の `.db` / `-wal` / `-shm` を揃えて元名に rename する。
- **seed / reset は対象クローンの app（predict / analyze / fetch 等）を停止してから実行する**。稼働中プロセスが開いている DB の `-wal`/`-shm` を退避・削除すると、そのプロセス側の DB 整合性を壊しうるため。
- `reset-db.sh` は primary clone（golden 元）の `data` を対象にすると既定で中断する（全クローンの seed 元を失うため）。意図的に primary を reset するときのみ `--force` を付ける。

### マイグレーション注意

`results.status` カラム (migration `20260427000002`) は `NOT NULL DEFAULT 'finished'`。
既存 DB に当てると過去レコードは全て `finished` になるため、競走除外馬が混在していた場合は誤情報になる。
status 情報を正確に取り直すには対象 PDF を再 ingest する（同じ `race_id` の UPSERT で全フィールドが上書きされる）。

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

## 出馬表を取り込む

JRA 出馬表 PDF（`N回VENUE日出馬表.pdf`）から枠番・馬番・馬名・騎手を取り込む。

```bash
cargo run -p parse-entries -- pdfs/entries/inbox/20260419-03nakayama08.pdf
# ingested: 12 race card(s), 162 horse entry/entries from ...
# moved: pdfs/entries/inbox/... -> pdfs/entries/done/...
```

`pdfs/entries/inbox/` に置いた PDF は取り込み成功後に `pdfs/entries/done/` へ自動移動する。

取り込み後の確認:
```bash
sqlite3 data/paddock.db "SELECT race_id, venue, race_num, distance, surface FROM race_cards ORDER BY race_num;"
sqlite3 data/paddock.db "SELECT gate_num, horse_num, horse_name, jockey FROM horse_entries WHERE race_id='2026-3-nakayama-8-1R' ORDER BY horse_num;"
```

## netkeiba から取得する

JRA 成績 PDF が未公開の当日・直近レースを予想するためのデータ源。出馬表・オッズ・近走・確定結果を
netkeiba から取得して DB に入れる。netkeiba は第三者の公開サーバなので、リクエスト間隔（`--interval`）で
礼節に留意する。

### 出馬表＋オッズ＋近走（`fetch-card`）

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

### 出走馬の近走（`fetch-history`）

出馬表から各馬の `horse_id` を引いて近走を取得し、`results` に取り込む（前走フォーム等の予想シグナル用）。

```bash
cargo run -p fetch-history -- 202606030801 202606030802   # race_id（複数可・中山3回8日 1R/2R）
cargo run -p fetch-history -- --horse-id 2021104500       # 出馬表をバイパスして horse_id 直指定
cargo run -p fetch-history -- 202606030801 --no-backfill  # pdf 成績行への horse_id backfill を抑止
```

### 確定結果で results を再取込（`fetch-results`）

netkeiba 結果ページから既存 `results` を再取得し、`jockey` / `trainer` を netkeiba の略名表記に揃える。
PDF 由来の馬主名混入や調教師名の不一致を解消し、predict の entry↔results join を噛み合わせる
（`races` 行は更新しない）。

```bash
cargo run -p fetch-results -- --from 2026-01-01 --to 2026-03-31
cargo run -p fetch-results -- --from 2026-06-01 --interval 1500
```

## 買い目シミュレーション（`simulate`）

買い目ポートフォリオの収支シミュレータ。全着順を列挙して払戻・収支を集計する。

```bash
cargo run -p simulate -- --input bets.json            # 買い目定義 JSON
cargo run -p simulate -- --input bets.json --main 5-1-3   # 本線の着順を上書き
echo '{ ... }' | cargo run -p simulate                   # 標準入力からも可
```

## 予想ワークフロー全体像

```
fetch-card        … 出馬表＋オッズ＋近走を取得（race_cards / race_odds / horse_past_runs）
  └─ (fetch-history で近走を results に補強)
analyze predict   … 1 レースの確率を確認（単発・DB 変更なし）
predict           … 1 日分を対話的に予想・購入記録（--budget で開始）
  └─ レース確定後 → fetch-results で結果を取り込み、predict --settle で自動精算
analyze backtest  … 過去レンジで予想ロジックを walk-forward 検証
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
├── use-case/                 Repository / PdfParser / EntryParser トレイト
├── interface/
│   ├── pdf-parser/           mutool + OCR ハイブリッドで PDF→Race（成績）
│   ├── pdf-ocr/              tesseract サブプロセスで PDF→OCR 行
│   ├── entry-parser/         mutool stext.json で PDF→RaceCard（出馬表）
│   └── rdb-gateway/          sqlx-sqlite で Repository 実装
├── infrastructure/
│   └── config/               環境変数から Config を構築
└── apps/
    ├── parse-pdf/            CLI バイナリ: 成績 PDF 取り込み
    ├── parse-entries/        CLI バイナリ: 出馬表 PDF 取り込み
    ├── analyze/              CLI バイナリ: horse / course / jockey / trainer / predict / backtest
    ├── predict/              CLI バイナリ: 予想セッション（対話）
    ├── fetch-card/           CLI バイナリ: netkeiba 出馬表＋オッズ＋近走を取得
    ├── fetch-history/        CLI バイナリ: netkeiba 近走を results に取得
    ├── fetch-results/        CLI バイナリ: netkeiba 確定結果で results を再取込
    └── simulate/             CLI バイナリ: 買い目ポートフォリオの収支シミュレータ
```

## 既知の制約

- 斤量・人気・調教師・騎手は OCR 非依存で決定的に取得する。
  - 斤量は CID 数字で読めるため `mutool -F stext.json` の座標索引から確定する（EdiF 復号不要。当初「斤量も EdiF」という前提は現行 PDF 形式では誤りだった。#124 / ADR 0018）。
  - 人気は単勝オッズの昇順順位から算出する（EdiF の人気列を復号せずに決定的・正確。#124 / ADR 0018）。同オッズは同順位。
  - 調教師・騎手も stext 座標索引から列単位で抽出する（#124 以前から存在する `jockey_stext` 由来。ADR 0018 のスコープ外）。
  - 着順は mutool が**行順**（完走順）から推定するのが土台。OCR 結果が完全集合として信頼できる場合のみ OCR 由来の値で上書きし、信頼できない（集合が不完全等）場合は行順 fallback を維持する。
  - **上り3F** は依然 EdiF で未対応（現状 `HorseResult` に該当フィールドが無い）。
- 競走除外・出走取消・競走中止の馬（完走しなかった馬）は `finishing_position = NULL` で保存される。
- タイム・オッズは mutool テキスト抽出ベースで取得（同サンプル PDF で取得率は概ね 8〜9 割超）。同タイム馬（`〃` 表記）はタイムが取れない。
- 騎手を座標索引で抽出するのは、テキスト抽出だと隣の馬主・調教師名や斤量が騎手列に連結するため。騎手名・馬名は取り込み・検索の双方で共通の正規化（全角英字/数字→半角、全角ピリオド `．`→`.`、半角カナ→全角）が適用され、`Ｃ．ルメール` でも `C.ルメール` でも、`ｲｸｲﾉｯｸｽ` でも `イクイノックス` でも引ける。
  - 既存 DB に古い（汚染された）騎手値が残っている場合は、対象 PDF を再 ingest すると `jockey` 列が更新される。
- 馬名・騎手名は **部分一致（中間一致）** で検索でき、入力は正規化されるため表記ゆれを吸収する（`analyze` の `horse` / `jockey`）。複数候補がヒットした場合は候補一覧を提示するので、絞り込んで再実行する。既存 DB に未正規化の値が残る場合は対象 PDF を再 ingest すると揃う。

## ライセンス

MIT — `LICENSE` 参照。
