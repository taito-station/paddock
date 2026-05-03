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
- **tesseract**（任意・推奨）: 画像 OCR で mutool のテキスト抽出が拾えないカラム（着順・斤量・人気・調教師）を補完する。インストールされていれば自動的にマージし、未インストールなら warn ログを出して mutool 単独データのみで完了する（破壊的ではなく加算的）。
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
cargo run -p parse-pdf -- pdfs/inbox/2026-3nakayama6.pdf
```

複数ファイルを並列で取り込む（既定は CPU コア数まで同時実行）:
```bash
cargo run -p parse-pdf -- pdfs/inbox/*.pdf
cargo run -p parse-pdf -- -j 4 pdfs/inbox/*.pdf   # 並列度を明示
```

`pdfs/inbox/` 配下のファイルを引数にした場合、取り込みが成功した PDF は `pdfs/done/` へ自動的に移動される（未取り込みファイルが一目で分かるようにするため）。`samples/` などインボックス外のパスは移動されない。複数ファイル指定時、1 件でも失敗があれば終了コードが非 0 になる（成功分の取り込みと移動は維持される）。

### 抽出ロジック

抽出は常に **mutool テキスト抽出 + OCR 補完** のハイブリッド方式で動作する（モード切替なし）。

1. `mutool draw -F text` で PDF テキストを抽出し、土台となる Race / 結果テーブルを構築
2. tesseract が利用可能なら PDF を PNG 化して OCR をかけ、mutool で `None` のカラム（着順・斤量・人気・調教師）を additive に補完
3. tesseract が見つからない／OCR が失敗した場合は warn ログを出し、mutool 単独のデータで完了（既存データを破壊しない）
4. 着順は OCR 抽出結果が「1〜頭数の完全集合の半分以上を占める」場合のみ採用、そうでなければ mutool の行順 fallback を使う

進捗は `RUST_LOG=info` で OCR 開始・終了・所要時間が source 別に表示される:

```bash
RUST_LOG=info cargo run -p parse-pdf -- pdfs/inbox/2026-3nakayama6.pdf
# INFO ingest{source=pdfs/inbox/2026-3nakayama6.pdf}: ocr starting race_count=12 bytes=696311
# INFO ingest{source=pdfs/inbox/2026-3nakayama6.pdf}: ocr extracted, applying merge pages=7 elapsed_ms=55114
# INFO ingest{source=pdfs/inbox/2026-3nakayama6.pdf}: ocr merge complete
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
├── use-case/                 Repository / PdfParser トレイト＋Interactor
├── interface/
│   ├── pdf-parser/           mutool サブプロセスで PDF→Race 構造体（HybridParser も同梱）
│   ├── pdf-ocr/              tesseract サブプロセスで PDF→OCR 行（着順以外の補完用）
│   └── rdb-gateway/          sqlx-sqlite で Repository 実装
├── infrastructure/
│   └── config/               環境変数から Config を構築
└── apps/
    ├── parse-pdf/            CLI バイナリ: PDF 取り込み
    └── analyze/              CLI バイナリ: horse / course / jockey
```

## 既知の制約

- JRA PDF は着順・斤量・人気・調教師などのカラムが EdiF カスタムフォントで描画されており、mutool テキスト抽出では拾えない。
  - 着順は mutool が常に**行順**から推定する（土台）。
  - tesseract がインストールされていれば OCR で斤量・人気・調教師を補完し、着順も OCR 結果が完全集合として信頼できる場合は OCR 由来の値で上書きする。
  - tesseract が無い場合や OCR が失敗・不完全な場合は、mutool 単独のデータで保存する（warn ログのみ）。
- 競走除外・出走取消の馬は `finishing_position = NULL` で保存される。
- タイム・オッズ・騎手は mutool テキスト抽出ベースで取得（同サンプル PDF で取得率は概ね 8〜9 割超）。同タイム馬（`〃` 表記）はタイムが取れない。
- 馬名・騎手名は完全一致で検索する（部分一致／カタカナ正規化等は未対応）。

## ライセンス

MIT — `LICENSE` 参照。
