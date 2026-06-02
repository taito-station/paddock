## Summary
- JRA 公式のオッズ（単勝/複勝/馬連/馬単/三連複/三連単）を取得する interface クレート `odds-scraper` を新規追加
- Domain に `RaceOdds` アグリゲートと値オブジェクト（`BetType`/`OddsValue`/`PlaceOdds`/組番キー）、Use-Case に port `OddsScraper` を追加
- HTML パーサ + 純粋関数 `assemble` を検証済みコアとし、fixture で6馬券種すべてのパースをテスト

## 設計判断（ADR 0001）
- **JRA オッズは race_id ベースの GET URL では取得不可。** `accessO.html` への `cname` トークン POST 遷移が必須で開催日限定。ライブ遷移層は best-effort とし、検証は保存 HTML fixture に対するパース／組み立てで担保（既存 PDF パーサと同方針）
- **HTTP は ureq に統一**（issue 本文の reqwest 指定から変更）。既存プロジェクトの同期 HTTP・同期 port と一貫させるため
- 既存 `Interactor` には未配線（単独 port として公開、アプリ配線・DB 永続化はスコープ外）

## 成果物リンク
- [ADR 0001: JRA オッズスクレイパー](https://github.com/taito-station/paddock/blob/feat/issue-10-jra-odds-scraper/docs/adr/0001-jra-odds-scraper.md)

## Test plan
- [x] `cargo test -p odds-scraper`（パーサ／assemble の統合テスト 7 件 PASS）
- [x] `cargo test -p paddock-domain -p paddock-use-case`（非回帰）
- [x] `cargo clippy -p odds-scraper -p paddock-domain -p paddock-use-case --all-targets -- -D warnings`（変更クレートはリント clean）
- [x] `cargo build --workspace`

> 注: `cargo clippy --workspace` は `entry-parser` テストの既存 `useless_vec` 指摘で失敗するが、本 PR の変更とは無関係（別ブランチ `feat/fetch-range` で対応済み）。ブラウザテストは対象外（Rust ライブラリ／CLI で画面なし）。

Closes #10

🤖 Generated with [Claude Code](https://claude.com/claude-code)
