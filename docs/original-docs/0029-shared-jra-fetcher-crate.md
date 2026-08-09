# ADR 0029: JRA fetcher を共有 crate `jra-fetcher` に集約 (Issue #155)

> 採番注記: 当初 `0022` で追加されたが [ADR 0022](0022-rest-api-read-server.md)（REST API read 基盤, Issue #33）と
> 番号が重複していたため、後発の本 ADR を `0029` にリナンバーした（2026-06-20）。内容に変更はない。

## ステータス
承認済み（採用）

## コンテキスト
JRA PDF 取得の `UreqFetcher` 実装が **2 箇所に重複・分岐**していた
（`pdf_parser::UreqFetcher`〔結果 PDF・バルク〕と `parse-entries` 内の `UreqFetcher`〔出馬表・単発〕）。
#152（PR #153）でタイムアウト＋リトライを入れた際、両者の差異がセルフレビューで繰り返し指摘された：

1. **エラー分類の非対称**: `PdfFetcher` トレイトは `paddock_use_case::Result` を返すため、結局どちらの取得
   失敗も `paddock_use_case::Error::Internal` に丸められていた（pdf_parser 側の `Error::Fetch` も
   use-case 境界で `Internal` 化）。`Internal` は内部バグ用の semantic で、外部 HTTP 由来の失敗を畳むと
   ログ/監視で切り分けられない。
2. **タイムアウト定数・Agent 構築の重複**: `CONNECT_TIMEOUT=10s` / `GLOBAL_TIMEOUT=60s` と Agent 構築が
   両クレートにコピーされ、値ズレが経路ごとの挙動差を生むリスクがあった。
3. **不在判定の非対称**: バルク経路は `403`/`404` 両方を不在とするが、entries は `404` のみだった。

## 決定
共有 crate **`src/interface/jra-fetcher`** を新設し、`JraFetcher` に取得ロジックを一本化する。

- `JraFetcher` が `paddock_use_case::PdfFetcher` を実装し、両アプリ（`parse-pdf` / `parse-entries`）が
  これを直接利用する。`pdf_parser` からは fetcher を撤去（`fetcher.rs`・re-export・`ureq` ランタイム依存・
  未使用化した `Error::Fetch` を削除）。`parse-entries` のローカル実装も削除。
- **集約する責務**: タイムアウト付き `ureq::Agent` 構築／タイムアウト定数／指数バックオフのリトライ
  （`is_transient` 分類含む）／不在判定（`403`/`404`）／`RateGate`（`--max-rps` ペーシング）。
  `JraFetcher::new(min_interval)` は単発呼び出し（entries）が `None`、バルク取得が `--max-rps` 由来の
  interval を渡す。
- **エラー分類の是正**: `paddock_use_case::Error` に `Fetch(String)` と `Timeout(String)` を追加し、
  ureq エラーを **timeout→`Timeout` / その他→`Fetch`** にマップする（`Internal` 丸めを廃止）。
- **不在判定の統一**: `fetch_if_exists` は `403`/`404` 両方を不在（`Ok(None)`）として扱う共通契約に統一する
  （seiseki と同契約）。**ただしこれは `fetch_if_exists` を呼ぶ経路の契約レベルの統一**であり、現状この
  メソッドを使うのは結果 PDF のバルク discovery（`interactor/pdf/fetch.rs`）のみ。entries の取り込み
  （`interactor/entry/ingest.rs`）は `fetch`（非 `if_exists`）を呼ぶため、entries の実挙動は不在判定統一の
  影響を受けず、`403`/`404` は従来どおり `Err` で表面化する。統一の実利は、将来 entries が discovery 的に
  `fetch_if_exists` を採用した時に一貫した不在判定を得られる点にある。実機で `403` を観測する必要は無く、
  共有 trait impl の契約統一を優先した。

## 理由
- 取得の挙動（タイムアウト・リトライ・不在判定）を**単一実装**に集約することで、経路ごとの差異と将来の
  二重メンテを排除できる。`pdf_parser` は本来パーサであり、fetcher が同居していたのは偶発的だった。
- `Fetch`/`Timeout` の専用バリアントにより、ネットワーク障害を内部バグと区別してログ/監視できる。
  既存の `paddock_use_case::Error` への variant 追加は後方互換（網羅 match は wildcard 付きのみで破綻しない
  ことをビルドで確認済み）。
- リトライ policy は #152 で `pdf_parser::UreqFetcher` に集約済みだったものを、本 crate へ持ち上げて
  entries 側にも一貫適用する位置づけ。

## 影響・トレードオフ
- 取得失敗の型が `Internal` から `Fetch`/`Timeout` に変わる（ハンドリングは `?` 伝播が主で、振る舞いの
  退行は無し。ビルド・テストで確認）。
- 依存方向: `jra-fetcher` は `paddock_use_case`（トレイト＋Error）に依存する。これは従来 `pdf_parser` が
  同トレイトに依存していたのと同じ向きで、既存アーキテクチャと整合。
- **テスト用の共有 fixture** `src/interface/sample_pdf_fixture.rs` は、サンプル結果 PDF を独自の最小 ureq
  agent で取得する設計を維持する（`#[path]` include の standalone ファイルで「新規 crate を増やさない」
  方針のため、テスト基盤を use-case 層へ結合させない）。よって `pdf-parser` は `ureq` を **dev-dependency**
  として保持する。本番取得経路の重複は解消済みで、本件はテスト専用の意図的な例外。
- 今後 odds/netkeiba 等の別取得経路が増えた場合も、HTTP 取得の共通化は `jra-fetcher`（または同種の
  共有 crate）へ寄せる方針とする。
