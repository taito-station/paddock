# ADR 0008: netkeiba を当日(出馬表・オッズ)データソースに採用 (Issue #28)

## ステータス
承認済み

## コンテキスト
当日（これから走る）レースの予想に必要な入力（出馬表・単勝オッズ・人気）を、現状の paddock は
自動で揃えられない。

- `parse-pdf fetch` は結果(seiseki) PDF 専用で、これから走るレースの出馬表は取得できない。
- 出馬表は `parse-entries` で扱えるが自動取得の口が無い。**JRA は出馬表を予測可能な固定 URL で
  配信していない**ため、当日分を自動取得できない。
- オッズの永続化(`race_odds`)が未実装で、`predict` のライブセッションは買い目推奨(EV・Kelly)を
  出せずスキップになる(#25)。

メモリ方針では「外部 API より自己完結する解を優先」だが、**当日入力は公式に自動取得する手段が存在しない**。
一方 netkeiba の出馬表ページ(`race/shutuba.html`、EUC-JP)は出走馬・単勝オッズ・人気が 1 ページに揃っており、
`predict` が必要とする `race_cards` と `race_odds` を一括で満たせる。

## 決定
1. **netkeiba を当日データソースとして採用する。** 公式の自動取得手段が無いため、現実的な代替として
   公開ページをスクレイピングする。既定ウェイトを入れ netkeiba 側へ配慮する。
2. **取得は新規アプリ `paddock-fetch-card`(`src/apps/fetch-card`)に集約する。** parse-pdf(fetch/ingest)・
   parse-entries と並ぶ対称構造とし、PDF 専用だった既存アプリの責務を汚さない。
3. **既存資産を再利用・拡張する。** HTTP は `ureq`(ADR 0001 の統一方針)、文字コードは
   `encoding_rs::EUC_JP`。`NetkeibaScraper` port に出馬表フル取得用メソッドを追加し、
   既存 `fetch_shutuba`(近走取得用、`RunnerRef` のみ返す) は壊さない。
4. **`race_odds` 永続化は券種非依存の汎用 1 テーブルで新設する。**
   `(race_id, bet_type, combination_key, odds, odds_high, popularity, fetched_at)`。#28 では単勝(win)のみ
   populate し、#38 の組合せ券種をマイグレーション再設計なしで受けられるようにする。オッズは変動するため
   常に最新値で upsert する。
5. **ドメイン(`RaceCard` / `RaceOdds`)は変更しない。** 出馬表の斤量・性齢は取得しても破棄する。
   人気は `race_odds` テーブルのカラムとして scrape 結果から直接保存する(ドメイン型は触らない)。

## 理由
- 当日入力に公式自動取得手段が無い以上、自己完結方針は「公式手段の範囲で完結」では達成できない。
  netkeiba 採用はこの制約下での現実的な選択であり、その位置づけを ADR に明記して方針との整合をとる。
- 新規アプリへの集約は、parse-pdf/parse-entries と同じ「1 アプリ 1 取得経路」のパターンを踏襲し、
  責務分離と将来拡張(#38/#40)の足場になる。
- 汎用 race_odds 表は、単勝のみの最小実装に比べ初期コストは僅かに増えるが、#38 でのスキーマ再設計・
  データ移行を回避できる。券種をキー(`bet_type` + `combination_key`)で正規化することで一様に扱える。
- ドメイン不変は、出馬表 PDF 取り込みや predict・backtest が依存する既存ドメインへの波及を避けるため。
  斤量・性齢の活用は #31 でドメイン拡張とあわせて設計する。

## 影響
- `race_odds` マイグレーション・`Repository::save_race_odds`・`NetkeibaScraper` の新メソッド・新規アプリ
  `fetch-card` を追加する。`Cargo.toml` の workspace members にアプリを登録する。
- スクレイピング依存のため、netkeiba 側の HTML 改変で parser が壊れうる。parse はフィクスチャ(保存 HTML)に
  対するユニットテストで決定論的に検証する(既存 PDF/オッズパーサと同方針)。
- 単勝のみ populate のため、`predict` の組合せ券種 EV は引き続き #38 待ち。本 ADR はその供給基盤を用意する。
- 出馬表 dedup は `fetch_history` を用い、オッズは upsert で最新化する二系統の更新方針となる。

## 関連
- 仕様書: [netkeiba 当日データソース取り込み](../specifications/netkeiba-datasource.md)
- ADR 0001(JRA オッズスクレイパ)/ ADR 0005(オッズ→predict 結線, #25)
- Issue: #28 / #38 / #40 / #31 / #25
