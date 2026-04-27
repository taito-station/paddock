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
- **tesseract**（`hybrid` パーサー使用時のみ）: 画像 OCR で着順以外の補完情報（斤量・人気・調教師）を取得
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

`pdfs/inbox/` 配下のファイルを引数にした場合、取り込みが成功した PDF は `pdfs/done/` へ自動的に移動される（未取り込みファイルが一目で分かるようにするため）。`samples/` などインボックス外のパスは移動されない。

### パーサーモード切替

環境変数 `PADDOCK_PARSER` で抽出方式を選択できる:

| 値 | 動作 | 必要環境 |
|---|---|---|
| `mutool`（既定） | mutool テキスト抽出のみ。高速・オフライン・ネット/OCR 依存なし | mupdf-tools |
| `hybrid` | mutool 抽出 + tesseract OCR で斤量・人気・調教師カラムを補完 | mupdf-tools + tesseract + tesseract-lang |

```bash
PADDOCK_PARSER=hybrid cargo run -p parse-pdf -- samples/2026-3nakayama6.pdf
```

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

- JRA PDF は着順・斤量・人気・調教師などのカラムが EdiF カスタムフォントで描画されており、テキスト抽出できない。
  - **mutool モード（既定）**: 結果テーブルの**行順**から着順を推定する。斤量・人気・調教師は欠落。
  - **hybrid モード（`PADDOCK_PARSER=hybrid`）**: tesseract で PNG 化したページを OCR し、斤量・人気・調教師を補完。着順は OCR が信頼できる場合のみ採用、ダメなら mutool の行順 fallback に戻る。
- 競走除外・出走取消の馬は `finishing_position = NULL` で保存される。
- タイム・オッズ・騎手は mutool テキスト抽出ベースで取得（同サンプル PDF で取得率は概ね 8〜9 割超）。同タイム馬（`〃` 表記）はタイムが取れない。
- 馬名・騎手名は完全一致で検索する（部分一致／カタカナ正規化等は未対応）。

## ライセンス

MIT — `LICENSE` 参照。
