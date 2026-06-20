# ADR 0026: OCR/PDF 統合テストを CI で実走する (mupdf バージョン固定) (Issue #172)

## ステータス
承認済み

## コンテキスト

#160（PR #169）の CI 新設時、`pdf-ocr` / `pdf-parser` の統合テスト（`tests/test_render.rs` / `tests/test_parse.rs`）は CI で実走できず、crate ごと `--exclude` した（OCR 非依存のユニットテストのみ `--lib` で実走）。Issue #172 で CI 再現可能化を図る。

調査（debian / ubuntu 各イメージで `samples/2026-3nakayama6.pdf` を実走）で、**失敗の主因は tesseract ではなく `mutool`（mupdf）のバージョン**だと判明した:

- `pdf-parser` の `MutoolParser` は `mutool draw -F text` / `-F stext.json`（mupdf）**のみ**を使い、tesseract は呼ばない。`pdf-ocr` の `render_pdf_to_pngs` も `mutool draw -F png` のみ。
- tesseract を実際に使うのは `pdf-ocr` の `OcrExtractor::extract`（`tests/test_extract.rs`）だが、これは遅いため既に `#[ignore]`。
- サンプル PDF の解析結果（12 レース・騎手名・調教師名・距離等の具体値アサーション）は mupdf のテキスト抽出出力に依存し、版が古いと **0 レース**になる:

| mupdf | 環境 | 結果 |
|---|---|---|
| 1.21.1 | debian bookworm（= importer runtime） | 0 レース・FAIL |
| 1.23.10 | ubuntu 24.04（= `ubuntu-latest`） | 0 レース・FAIL |
| 1.25.1 | debian trixie / ubuntu 25.04 | 全アサーション PASS |
| 1.27.2 | macOS dev（homebrew） | 全アサーション PASS |

すなわちパーサは mupdf **≥ 1.25** で互換。`ubuntu-latest` は apt で 1.23 しか入らないため、apt インストールだけでは再現できない（PR #169 で mupdf-tools を入れても失敗し除外した経緯と一致）。

## 決定

- `pdf-ocr` / `pdf-parser` の統合テストを、**mupdf 1.25 を持つ `debian:trixie-slim` コンテナ上の専用 CI ジョブ**（`ocr-pdf`）で実走する。
  - ジョブは `runs-on: ubuntu-latest` + `container: debian:trixie-slim`。apt で `mupdf-tools`（1.25.1）/ `tesseract-ocr` / `tesseract-ocr-jpn` / `build-essential`（rustls の ring が C/asm をビルドする）/ `git curl ca-certificates`（checkout・rustup ブートストラップ）を入れる。ureq は rustls（純 Rust TLS）で sqlx(native-tls) は閉包外のため `libssl-dev`/`pkg-config` は不要。
  - Rust toolchain は本体ジョブと同じ 1.96.0（`rust-toolchain.toml`）。`Swatinem/rust-cache` でキャッシュ。
  - `cargo test --locked -p pdf-ocr -p pdf-parser -- --test-threads=1`（lib + 統合）を実走する。Postgres は不要（これらは DB を触らない）。`--test-threads=1` は複数テストバイナリの並行 JRA 取得を避け出力を決定的にするため。
  - mupdf の版ドリフト対策として、テスト前に **`mutool` のバージョンが 1.25 以上であることを assert する gate ステップ**を置く。下限未満ならサイレントに 0 レース化させず明示的に fail させる。
- **`test_extract.rs`（tesseract OCR・低速）は `#[ignore]` のまま**とする。tesseract の版・言語データ差に依存し決定論性が低いため、CI 標準実行には載せない（必要時に `--ignored` で手動実行）。
- 本体 `ci` ジョブは従来どおり `--exclude pdf-ocr --exclude pdf-parser` を維持し、これまで本体に置いていた「OCR クレートの `--lib` のみ」ステップは新ジョブへ移管して重複を解消する。
- サンプル PDF は JRA 著作物で repo に含めない（gitignore）。CI ではフィクスチャが JRA 公式から取得を試み、取得できれば統合テストが実走、取得不可なら graceful skip（ユニットテストは PDF 不要で常時実走）。

## 理由

- **版固定がコンテナで最も再現可能**: `ubuntu-latest` の apt mupdf（1.23）は不足、ソースビルドは CI を重く・脆くする。`debian:trixie-slim` は apt 一発で 1.25 が入り、イメージタグで版を固定できる。
- **専用ジョブで分離**: 本体ジョブは Postgres サービス + `localhost` 前提。全体を container 化すると service ネットワーク（`localhost`→サービス名）や DB 接続を作り直す必要があり影響が広い。pdf テストは DB 不要なので別ジョブに切る方が安全・並列で速い。
- **`test_extract` を ignore 維持**: 真に tesseract 版依存なのはこのテストだけ。決定論を確保できないものを CI 標準に載せると flaky 化するため、対象外を継続する（Issue 要件の「OCR 統合テストを CI 実走」は mupdf 依存の render/parse を実走対象に戻すことで満たす）。
- **取得失敗を skip に倒す**: フィクスチャ既存設計（PDF 取得不可なら `None`→早期 return）を踏襲し、JRA 一時不通で CI を赤くしない。ユニットテストは常時カバレッジを担保する。
- **イメージは tag 参照（digest ピンしない）**: 外部 action は SHA ピンするが、コンテナイメージは本体  job の `postgres:17-alpine` と同様に tag 参照とする（OS イメージはセキュリティ更新を取り込みたく、digest 固定は陳腐化・手動更新の負担が大きい）。版ドリフトの実害（mupdf が下限割れ）は上記 assert gate で検知する。

## 影響

- CI に新ジョブ `ocr-pdf` が増える（コンテナ pull + 依存 apt + ビルド）。`rust-cache` 前提で 2 回目以降は短縮。
- dev（macOS mupdf 1.27.2）と CI（trixie mupdf 1.25.1）で mupdf 版が異なる。両版とも現アサーションを満たすことは確認済みだが、将来 mupdf 出力が変わればどちらかで割れうる。その場合はコンテナのイメージタグ更新かアサーション調整で対応する。
- importer runtime（`importer.Dockerfile`）は debian **bookworm**（mupdf 1.21）であり、`MutoolParser` 単体では 0 レースになる版。importer の解析経路（OCR ハイブリッド）への影響確認・bookworm→trixie 引き上げは本 Issue のスコープ外（別途要確認）として記録する。
- **JRA 取得不可日のカバレッジ低下**: サンプル PDF を repo に含めない設計上、JRA から取得できない run では統合テストがアサーション未実行のまま緑になる（`#[ignore]` ではなく早期 return のため）。ユニットテストは常時走るが、mupdf 依存の解析回帰は「取得成功した run」でのみ実証される。恒常的な実走保証が必要になれば、サンプルの別保管（暗号化アーティファクト等）を別 Issue で検討する。本 PR では Actions 実 run で 12 レース解析の pass ログを確認して実走を実証する。
- ADR は連番末尾 `0026` で採番（採番当時 `0022` が重複していたため。重複は後に是正済み＝後発の `jra-fetcher 集約` を ADR `0029` にリナンバー、2026-06-20）。
