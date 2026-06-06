# paddock

JRA 公式のレース成績 PDF をパースし、SQLite に蓄積して傾向を集計する CLI ツール（Rust 製）。

## 何ができるか

- 馬の特性集計: 芝/ダート、距離帯、枠順（内/中/外）、馬場状態ごとの勝率・連対率
- コースの特性集計: 競馬場 × 距離 × 芝ダ で、枠順別の勝率・連対率
- 騎手の基本スタッツ: 全体／芝ダ／枠順別の勝率・連対率

## 必要環境

- macOS (Apple Silicon / Intel)。Linux でも動作見込み。
- Rust ツールチェイン: `rust-toolchain.toml` で `1.93.0` 固定
- **mupdf-tools** (`mutool` バイナリ): PDF テキスト抽出に使用
  ```bash
  brew install mupdf-tools
  ```
  JRA の PDF は埋め込みフォントを使うため、純 Rust の `pdf-extract` では文字化けする。`mutool` をサブプロセス経由で呼び出して回避している。
- **tesseract** + **tesseract-lang**（jpn パック）: 画像 OCR で mutool のテキスト抽出が拾えないカラム（着順・斤量・人気・調教師）を補完する。`parse-pdf` 起動時に preflight チェックが走り、未インストール／jpn パック未導入ならその場でエラー終了する。
  ```bash
  brew install tesseract tesseract-lang   # 日本語パック含む
  tesseract --list-langs                  # jpn が表示されることを確認
  ```

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

末尾の引数を省くほど取得範囲が広がる。存在しない日次・開催は 404 で自動的に打ち切られる。

```bash
cargo run -p parse-pdf -- fetch --year 2026 --venue nakayama --round 3   # 3回中山の全日次
cargo run -p parse-pdf -- fetch --year 2026 --venue nakayama            # 中山の全開催回×全日次
cargo run -p parse-pdf -- fetch --year 2026                             # その年の全場×全回×全日
```

- 発見方式は単一指定と同じ「URL 構築＋存在確認」。日次は `day` を増やしながら、最初の 404 が出た時点でその開催回を打ち切る。開催回・競馬場も存在しない組合せは 404 でスキップする。
- 既に `fetch_history` にある開催は**スキップ**されるため、再実行すると差分のみ取得できる（定期実行と相性が良い）。`--force` で全件再取得。
- JRA への負荷に配慮し、リクエスト間に既定 1 秒のウェイトを入れる（`--interval <秒>` で調整、`0` で無効）。逐次取得。
- 最後に `ingested / skipped / not-found / failed` の件数サマリを表示する。途中でネットワーク等の失敗があっても列挙は継続し、`failed > 0` のときのみ終了コード非 0。

### 抽出ロジック

抽出は常に **mutool テキスト抽出 + OCR 補完** のハイブリッド方式で動作する（モード切替なし）。

1. 起動時に `tesseract` バイナリと jpn 言語パックの存在を preflight チェックし、欠けていれば即エラー終了する
2. `mutool draw -F text` で PDF テキストを抽出し、土台となる Race / 結果テーブルを構築
3. PDF を PNG 化して OCR をかけ、mutool で `None` のカラム（着順・斤量・人気・調教師）を additive に補完
4. 着順は OCR 抽出結果が「1〜頭数の完全集合の半分以上を占める」場合のみ採用、そうでなければ mutool の行順 fallback を使う

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

馬の傾向:
```bash
cargo run -p analyze -- horse "イクイノックス"
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

枠順は次のように 3 グループに集約:
- Inner: 1〜3 枠
- Middle: 4〜6 枠
- Outer: 7〜8 枠

## 予想セッション（対話）

1 日分のレースを順番に処理し、買い目推奨を確認しながら賭け金と払い戻しを記録する対話型 CLI。

```bash
# 新規開始（--budget 必須）
cargo run -p predict -- --date 2026-06-01 --budget 10000

# 中断したセッションを保存済みの残高から再開
cargo run -p predict -- --date 2026-06-01 --resume

# 収支サマリと買い目明細を表示（読み取り専用）
cargo run -p predict -- --date 2026-06-01 --summary
```

- 各レースで確率表と買い目推奨（期待値・Kelly 配分）を表示する。
- `[y=推奨通り / e=編集 / s=スキップ]` で購入方法を選び、レース後に**買い目ごとに**実際の払い戻し額を入力する（命中精度・回収率の分析に使う）。
- 推奨額は Kelly 配分を比例縮小方式で算出し、合計が残高を超えないよう収める。
- オッズ未取得（`race_odds` 未整備）のレースはスキップのみ受け付ける。
- 出馬表（`race_cards`）が取り込み済みであることが前提（「出馬表を取り込む」を参照）。

### セッションの永続化（`--resume` / `--summary`）

- セッションは **1 開催日 = 1 セッション**として `predict_sessions` に、購入した買い目は `predict_bets` に保存され、レース確定ごとに 1 トランザクションで更新する。
- `--resume`: 同日の未完了セッションを保存済み残高から再開し、購入済みレースはスキップする（スキップしただけのレースは再提示される）。完了済みセッションでは `--summary` を案内する。
- `--summary`: 開始予算・残高・総投資・総払戻・収支・回収率と買い目明細を表示する（DB を変更しない）。
- 既にセッションがある日に `--resume` なしで実行すると、二重作成を避けるため中止して `--resume` / `--summary` を案内する。

> オッズの永続化（`race_odds` テーブル）は別 Issue で対応予定。現状はオッズ未取得として扱われるため、ライブセッションではスキップのみになる。

## DB

- 既定パス: `data/paddock.db`
- スキーマは初回起動時に自動マイグレート（`deployments/db/migrations/`）
- 環境変数 `PADDOCK_DB_URL` で接続先を上書き可能（例: `sqlite://./other.db?mode=rwc`）
- DB を作り直したい場合は `data/paddock.db` を消してから取り込み直し

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
    ├── analyze/              CLI バイナリ: horse / course / jockey
    └── predict/              CLI バイナリ: 予想セッション（対話）
```

## 既知の制約

- JRA PDF は着順・斤量・人気・調教師などのカラムが EdiF カスタムフォントで描画されており、mutool テキスト抽出では拾えない。
  - 着順は mutool が常に**行順**から推定する（土台）。
  - OCR で斤量・人気・調教師を補完し、着順も OCR 結果が完全集合として信頼できる場合は OCR 由来の値で上書きする。
  - OCR が信頼できない（着順の集合が不完全等）場合は mutool の行順 fallback を維持する。
- 競走除外・出走取消の馬は `finishing_position = NULL` で保存される。
- タイム・オッズ・騎手は mutool テキスト抽出ベースで取得（同サンプル PDF で取得率は概ね 8〜9 割超）。同タイム馬（`〃` 表記）はタイムが取れない。
- 馬名・騎手名は完全一致で検索する（部分一致／カタカナ正規化等は未対応）。

## ライセンス

MIT — `LICENSE` 参照。
