# ADR 0001: JRA オッズスクレイパーの実装 (Issue #10)

## ステータス
承認済み（**ライブ遷移層 `UreqOddsScraper` / `odds-scraper` crate は #287 / ADR 0048 で撤去・supersede**。
live odds 取得は netkeiba の `OddsScraper` 実装に統一。本 ADR のパース設計記録は歴史的価値で残置）

## コンテキスト
issue #10 で、JRA 公式サイトから当日の馬券オッズ（単勝/複勝/馬連/馬単/三連複/三連単）を
取得する interface クレートが求められた。

調査の結果、JRA のオッズページには以下の本質的制約があることが判明した。

- オッズ画面は race_id を含む安定した GET URL では取得できない。結果 PDF
  （既存 `pdf-parser`）のような直接 URL が存在しない。
- `accessO.html` は GET だとエラーページへ 301 リダイレクトする。
- オッズ画面への遷移は、メニューページの JavaScript リンク
  `doAction('/JRADB/accessO.html', '<cname>')` が持つ **`cname` セッショントークン**を
  `accessO.html` へ POST することで初めて辿れる。
- 開催日（週末・祝日）以外はライブのオッズページ自体が存在しない。

このため「ライブ遷移層」はテストが困難かつ本質的に不安定であり、検証可能な形で
実装範囲を切り分ける必要があった。

## 決定
1. **HTTP クライアントは `ureq`（同期）に統一する。** issue 本文では reqwest が
   挙げられていたが、既存プロジェクトは `UreqFetcher` をはじめ全て同期 `ureq` に
   統一されており、port トレイト（`PdfFetcher` 等）も同期である。二系統の HTTP
   スタック・async ランタイム混在を避け、一貫性を優先する。
2. **レイヤー配置**（依存方向 Apps → Interface → Use-Case → Domain を厳守）:
   - Domain: `odds` モジュール。`BetType` / `OddsValue` / `PlaceOdds` /
     組番キー（`Pair` / `OrderedPair` / `Triple` / `OrderedTriple`）と、馬券種ごとの
     オッズマップを束ねるアグリゲート `RaceOdds`。
   - Use-Case: port トレイト `OddsScraper`（`scrape(&RaceId) -> Result<RaceOdds>`）。
   - Interface: 新規クレート `odds-scraper`。HTML パーサ（`scraper` クレート）と
     ライブ遷移層 `UreqOddsScraper`。
3. **検証は保存 HTML fixture に対するパーサ／組み立て（`assemble`）で行う。**
   ライブの POST/cname 遷移は best-effort 実装とし、純粋関数 `assemble` を
   検証済みコアとして切り出す（既存 PDF パーサと同方針）。
4. **既存 `Interactor<R, P, F>` には追加しない。** port は単独トレイトとして公開し、
   将来 interactor/app から消費する。本 issue ではアプリ配線・DB 永続化はスコープ外。

## 理由
- JRA の POST/cname 遷移はサイト改変で壊れやすく、開催日限定でしか実地検証できない。
  価値の中心であるパース／ドメイン変換ロジックを純粋関数として切り出すことで、
  fixture により決定論的に検証できる。
- ureq 統一は in-house の一貫性方針に沿い、依存とランタイムの単純化に寄与する。
- `Interactor` への追加は全 app に DI 強制が波及するため、未配線の段階では避ける。

> **追記（Issue #25 / ADR 0005）**: 本 ADR でスコープ外とした「アプリ配線」は #25 で実施した。
> `odds-scraper` は predict から消費されるようになり、下記 members への明示登録の例外は解消した
> （決定 #4「メイン Interactor に追加しない」は専用 `OddsInteractor` で踏襲）。

## 影響
- ~~`odds-scraper` は現時点でどのバイナリからも参照されないため、ワークスペースの
  ビルド／テストグラフに含めるべく `Cargo.toml` の `members` に明示的に登録した
  （「members はバイナリのみ」という通常方針に対する明示的な例外）。~~
  → #25 で predict が path 依存で参照するようになり、members 明示登録は撤去した。
- fixture は JRA の公開オッズテーブル構造を代表する形で作成しており、ライブページ
  との完全一致は将来のライブキャプチャで突き合わせる必要がある（精度は暫定）。
  これは既存パーサの既知制約と同様の扱い。
- ライブ遷移層（cname トークン抽出・POST）は実地未検証であり、開催日に実データで
  突き合わせる作業が残る。
