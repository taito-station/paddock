# ADR 0005: predict にオッズを結線し買い目算出を可能にする (Issue #25)

## ステータス
承認済み

## コンテキスト
`predict` は各馬の確率表を表示できるが、全レースで「オッズ未取得 — スキップ」となり
買い目（EV・Kelly 配分）が一切算出されなかった。原因は、オッズ取得経路が
`Interactor::race_odds` → `Repository::find_race_odds` のみで、その実装がスタブ
（常に `None`）だったため（`race_odds` テーブル・migration は未存在）。

`#10`（ADR 0001）で `interface/odds-scraper` クレートと `OddsScraper` トレイト
（`scrape(&RaceId) -> Result<RaceOdds>`、都度スクレイプ・キャッシュなし）は実装済み
だが、predict のフローに結線されていなかった。ADR 0001 はアプリ配線・DB 永続化を
**別 Issue のスコープ**として明示的に先送りしており、本 issue がその配線にあたる。

選択肢は以下だった:

- **案A（オンデマンド）**: predict に `OddsScraper` を DI し、レース処理時に live スクレイプする。
  ADR 0001 の「都度スクレイプ・キャッシュなし」設計と整合し、事前予想にそのまま使える。
- **案B（DB 永続化）**: `race_odds` テーブルを追加 → スクレイプ結果を保存 →
  `find_race_odds` を SELECT 実装に差し替える。過去レースの再現・履歴に向く。

## 決定
**案A（オンデマンド）を採用する。**

1. **専用 interactor `OddsInteractor<O: OddsScraper>` を新設**する（`src/use-case/src/interactor/odds/`）。
   `race_odds(&RaceId) -> Result<Option<RaceOdds>>` を提供し、`OddsScraper::scrape` を都度呼ぶ。
   - スクレイプ失敗（サイト改変・開催日外・ネットワーク等）→ warn ログを出して `None`
   - 取得成功だが全馬券種が空（未公開）→ `None`
   - いずれも predict 側でスキップ扱いになり、1 レースの失敗でセッション全体を止めない。
2. **メイン `Interactor<R, P, F>` には `OddsScraper` を足さない**（ADR 0001 決定 #4 を踏襲）。
   `HorseHistoryInteractor<R, S>`（`#37`）と同じく、スクレイパー依存の関心事は専用 interactor に
   切り出し、全 app への DI 強制を避ける。
3. **predict に `UreqOddsScraper` を DI** する。`App` が `OddsInteractor<UreqOddsScraper>` を保持し、
   `session.rs` は `app.odds.race_odds(&race_id)` を呼ぶ。
4. **dead code を撤去**する。スタブ化していた `Interactor::race_odds` /
   `Repository::find_race_odds`（トレイト・rdb-gateway 実装・スタブファイル）を削除する。
   DB 永続化（案B）は将来必要になった時点で別 Issue として再導入する。
5. `odds-scraper` は predict（バイナリ）から path 依存で参照されるようになったため、
   workspace `members` への明示登録（ADR 0001 で追加した例外）を解消する
   （`netkeiba-scraper` と同じ扱い）。

## 理由
- ADR 0001 が確立した「都度スクレイプ・キャッシュなし」設計と一直線でつながり、追加の
  スキーマ・保存タイミング・鮮度管理を持ち込まない（シンプル第一）。事前予想にそのまま使える。
- 案B はオッズの保存タイミング・無効化ポリシー・スキーマ設計を要し、本 issue の主目的
  （結線して買い目を出す）に対して過剰。履歴再現が必要になった時点で独立に導入できる。
- 専用 interactor 方式は `#37` で確立済みの前例に倣い、メイン Interactor のジェネリクスを
  全 impl ブロック・全 app に波及させずに済む。

## 影響
- `predict` はオッズが取得できるレースで `select_bets` が走り、EV 閾値超の買い目が
  推奨額付きで表示される。スクレイプ失敗・未公開オッズは従来どおり安全に skip 扱い。
- ライブ遷移層（cname トークン抽出・POST）は ADR 0001 のとおり実地未検証であり、
  開催日以外はオッズページ自体が存在しない。このため off-race-day の予想や CI では
  オッズは取得されず全レース skip になる（パニックせず継続する設計）。
- `race_odds` の永続化を撤去したことで、過去レースのオッズ再現は現時点でできない
  （案B 相当を将来 Issue で扱う）。
- オッズ依存の CLI テストケース（TC-10 / TC-12 / TC-15 / TC-16）は、決定論的な手動
  INSERT が使えなくなり、ライブ開催日でのみ実地確認となる（テストケース文書を更新）。

## 関連
- ADR 0001（JRA オッズスクレイパー実装, #10）
- 設計書 `docs/specifications/predict-session.md`
