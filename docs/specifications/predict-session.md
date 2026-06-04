# predict バイナリ: 対話型レーシングセッション

Issue #13

## 概要

1 日分のレースを順番に処理する対話型 CLI バイナリ `paddock-predict` を実装する。  
ユーザーは買い目推奨を確認しながら賭け金と払い戻しを記録し、1 日を通した残高管理を行う。

![predict 対話セッションフロー](diagrams/predict-session-flow.svg)

---

## CLI インターフェース

```
paddock-predict --date <YYYY-MM-DD> --budget <金額>
```

| オプション | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `--date`  | `NaiveDate` | ○ | 対象開催日（例: `2026-06-01`） |
| `--budget` | `u64` | ○ | 初期予算（円単位、例: `10000`） |

### 終了コード

| コード | 意味 |
|--------|------|
| 0 | 正常終了 |
| 1 | DB 接続エラー / 不正な引数 |

---

## UX フロー

### 起動

```
$ paddock-predict --date 2026-06-01 --budget 10000

=== 2026-06-01 開催 — 6 レース ===
初期予算: ¥10,000
```

### レース単位の対話ループ

```
--- レース 1: 東京 芝 1600m ---
残高: ¥10,000

馬番  馬名              勝率    連対率  複勝率
   1  アイネスフウジン   18.2%   35.1%   52.3%
   2  ダイナコスモス     12.4%   24.8%   38.7%
   ...

【買い目推奨】
  馬連  1-3   EV=1.42  Kelly=15%  推奨額=¥1,500
  馬単  1→3   EV=1.28  Kelly=8%   推奨額=¥800
  単勝  1     EV=1.15  Kelly=5%   推奨額=¥500

購入方法を選んでください [y=推奨通り / e=編集 / s=スキップ] > y

>>> レース後 <<<
実際の払い戻し額を入力 (なし: Enter のみ) > 4200

  賭け金: ¥2,800  払戻: ¥4,200  (+¥1,400)
残高: ¥11,400
```

### e（編集）モード

```
購入方法を選んでください [y=推奨通り / e=編集 / s=スキップ] > e

  馬連 1-3  推奨¥1,500  入力額 > 1000
  馬単 1→3  推奨¥800    入力額 > 0
  単勝 1    推奨¥500    入力額 > 500

>>> レース後 <<<
実際の払い戻し額を入力 > ...
```

金額に `0` を入力するとその買い目をスキップ。

### 一日集計

```
=== 2026-06-01 終了 ===
総賭け金:  ¥12,300
総払戻:    ¥15,600
最終残高:  ¥13,300
P&L:       +¥3,300
```

---

## アーキテクチャ

### 新規バイナリ

```
src/apps/predict/
├── Cargo.toml
└── src/
    ├── bin.rs       # エントリポイント、tokio::main
    ├── cli.rs       # clap 引数定義
    ├── session.rs   # 対話セッションループ
    └── setup.rs     # DI 構築（analyze と同パターン）
```

`Cargo.toml` の `[[bin]]` 名は `paddock-predict`。  
ワークスペース `Cargo.toml` の `members` に `"src/apps/predict"` を追加する。

### セッション状態（App 層）

```rust
struct SessionState {
    budget: i64,       // 現在残高（円）
    total_bet: i64,    // 累計賭け金
    total_payout: i64, // 累計払い戻し
}
```

セッション状態はアプリ層でのみ管理し、Domain / Use-Case 層には持ち込まない。

### 依存関係

```
src/apps/predict
    → paddock-use-case  (Interactor)
    → paddock-domain    (select_bets / estimate_probabilities — 純粋関数)
    → rdb-gateway       (Repository 実装)
    → paddock-config    (環境変数)
```

---

## 新規 Repository メソッド

`src/use-case/src/repository.rs` の `Repository` トレイトに以下を追加する。

```rust
/// 指定日に開催されるレース一覧を race_num 昇順で返す。
fn find_races_by_date(
    &self,
    date: NaiveDate,
) -> impl Future<Output = Result<Vec<Race>>> + Send;

/// race_id に対応するオッズを返す。未取得の場合は None。
fn find_race_odds(
    &self,
    race_id: &RaceId,
) -> impl Future<Output = Result<Option<RaceOdds>>> + Send;
```

### RDB 実装

| テーブル | SQL の概要 |
|---------|-----------|
| `races` | `WHERE date = $1 ORDER BY race_num ASC` |
| `race_odds`（新規または既存） | `WHERE race_id = $1 LIMIT 1` |

> **前提**: オッズは `paddock-odds-scraper`（Interface 層）が `race_odds` テーブルに保存済みであること。  
> オッズ未取得レースは `find_race_odds` が `None` を返し、「オッズ未取得」旨を表示してスキップ推奨とする。

---

## 新規 Use-Case インタラクターメソッド

`src/use-case/src/interactor/` 配下に以下を追加する（または既存ファイルに追記）。

```rust
// interactor/race/races_by_date.rs
impl<R: Repository> Interactor<R> {
    pub async fn races_by_date(&self, date: NaiveDate) -> Result<Vec<Race>> { ... }
}

// interactor/race/race_odds.rs
impl<R: Repository> Interactor<R> {
    pub async fn race_odds(&self, race_id: &RaceId) -> Result<Option<RaceOdds>> { ... }
}
```

`predict_race`（確率推定）は既存メソッドをそのまま再利用する。

---

## オッズなし時の動作

`find_race_odds` が `None` を返した場合:

1. 「オッズ未取得 — スキップ推奨」を表示する
2. `select_bets` には空の `RaceOdds` を渡す（推奨なしとなる）
3. ユーザーが `s` を選択してスキップできる

---

## ADR

[ADR-0004](../adr/0004-predict-session-binary.md)
