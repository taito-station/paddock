# predict バイナリ: 対話型レーシングセッション

[Issue #13](https://github.com/taito-station/paddock/issues/13)

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
| 0 | 正常終了（開催なし日付を含む） |
| 1 | DB 接続エラー / 実行中の DB I/O・クエリエラー |
| 2 | 引数パースエラー（`--date` / `--budget` の形式不正等） |

- 「開催なし日付」は異常ではないため exit code 0 とし、案内メッセージは **stdout** に出力する。
- 引数の形式不正（不正な日付・非数値の budget 等）は clap が自動で stderr にエラーを出力し **exit code 2** で終了する（既存 `analyze` バイナリと同じ `clap::Parser` 構成のため）。exit 1 はアプリ内部の DB エラーに限定する。

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

選択肢の意味:

| キー | 意味 | 動作 |
|------|------|------|
| `y` | 推奨通り購入 | Kelly 配分で算出した推奨額をそのまま確定する |
| `e` | 金額を編集 | 各買い目の金額を対話入力する（`0` 入力でその買い目をスキップ） |
| `s` | スキップ | このレースは購入せず賭け金 ¥0 で次へ進む |

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

### 残高ガード

賭け金（`y` の推奨額合計、または `e` の入力額合計）が **現在残高を超える場合は確定できない**。

- `y`: 推奨額合計 > 残高 のとき、その旨を表示して `e`（編集）または `s`（スキップ）に誘導する
- `e`: 各買い目の入力時点で「残り賭け可能額」を表示し、累計が残高を超える入力は再入力を促す

これにより残高は常に 0 以上に保たれる（後述の `SessionState` を `u64` で表現できる根拠）。

### 一日集計

```
=== 2026-06-01 終了 ===
総賭け金:  ¥12,300
総払戻:    ¥15,600
最終残高:  ¥13,300
P&L:       +¥3,300
```

`P&L = 総払戻 − 総賭け金`（= 最終残高 − 初期予算 と常に一致する）。  
ここで「総賭け金」は **実際に budget から減算した確定額の累計**であり、推奨額そのものではない（残高ガードや端数切り捨て後の額）。確定額を積算する限り上記の恒等式は常に成立する。

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
    budget: u64,        // 現在残高（円）— 残高ガードにより常に 0 以上
    total_bet: u64,     // 累計賭け金（実際に budget から減算した確定額の累計）
    total_payout: u64,  // 累計払い戻し
}
```

- CLI の `--budget`（`u64`）をそのまま初期 `budget` に代入するため型変換は不要
- 賭け金は残高ガードにより `budget` を超えないため、`budget -= bet` で桁あふれ（underflow）は発生しない
- `total_bet` は推奨額ではなく **実際に確定して budget から引いた額**を加算する（端数・ガード適用後の額）
- セッション状態はアプリ層でのみ管理し、Domain / Use-Case 層には持ち込まない

### 依存関係と呼び出し責務

```
src/apps/predict
    → paddock-use-case  (Interactor 経由: predict_race / races_by_date / race_odds)
    → paddock-domain    (App 層が直接呼ぶ純粋関数: select_bets)
    → rdb-gateway       (Repository 実装を Interactor に注入)
    → paddock-config    (環境変数)
```

呼び出し責務を明確化する:

- **確率推定・レース一覧・オッズ取得**（IO を伴う）は **Use-Case の Interactor 経由**で呼ぶ
- **`select_bets`**（IO なしの純粋関数）は **App 層（`session.rs`）が `paddock-domain` から直接呼ぶ**。Use-Case にラッパーを置かない（薄い委譲を増やさないため）
  - 実シグネチャは全引数が参照: `select_bets(probabilities: &[HorseProbability], race_odds: &RaceOdds, config: &BettingConfig) -> Vec<BettingRecommendation>`。呼び出しは `select_bets(&probs, &odds, &BettingConfig::default())`

### DI 構築（setup.rs）

既存の `Interactor` は `Interactor<R: Repository, P: PdfParser, F: PdfFetcher>` の 3 ジェネリクスを持つ。  
`paddock-predict` は PDF 解析・取得を使わないため、`analyze` バイナリと同様に **`UnusedParser` / `UnusedFetcher`（no-op 実装）を注入**して `Interactor` を構築する。

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

### `Option<RaceOdds>` を返すことについて

Domain には既に `RaceOdds::empty()` / `RaceOdds::is_empty()` があり、`select_bets` は空の `RaceOdds` に対して空 Vec を返す。  
それでも `find_race_odds` の戻り値を `Option` にするのは、**「オッズ未取得（`None`）」と「取得済みだが対象馬券が空（`empty`）」を区別するため**。前者はスキップ推奨を表示し、後者は推奨なしとして通常フローを進める。

### `Race` を返すことについて

`Race` は `results: Vec<HorseResult>` を持つが、予想フェーズ（レース確定前）では `results` は空である。  
レースヘッダ表示に必要なのは `venue` / `surface` / `distance` / `race_num` のみで、これらは `Race` に含まれる。  
`find_races_by_date` の SQL は **`results` を JOIN せず常に空 Vec で返す**（予想用途では結果は不要）。over-fetching は発生しないため専用 DTO は定義せず `Race` をそのまま返す。

### RDB 実装

| テーブル | SQL の概要 |
|---------|-----------|
| `races` | `WHERE date = $1 ORDER BY race_num ASC`（`results` は読み込まない） |
| `race_odds`（**本 Issue では未存在**、後述） | `WHERE race_id = $1 LIMIT 1` |

> **オッズ永続化のスコープ外注意**: 現状リポジトリに `race_odds` テーブルのマイグレーションは存在せず、`odds-scraper` もオンデマンド取得のみで永続化していない。  
> `race_odds` テーブルの追加とスクレイパーによる永続化は **別 Issue のスコープ**とする。  
> 本 Issue では `find_race_odds` が **常に `None` を返す実装でも成立する**よう、オッズ未取得時のフロー（次節）を必ずハンドリングする。  
> このため、オッズ・推奨を前提とするテスト（TC-10 / TC-12 / TC-15 / TC-16）は `race_odds` テーブル追加までモックまたは手動 INSERT を前提とする（[テストケース](../../tests/cli-test-cases/predict-command.md)の共通前提を参照）。

---

## 新規 Use-Case インタラクターメソッド

`src/use-case/src/interactor/race/` 配下に以下を追加する。  
いずれも既存 `Interactor<R, P, F>` のメソッドとして `impl` ブロックに追加する（ジェネリクス束縛は既存と同一）。

```rust
// interactor/race/races_by_date.rs
impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    pub async fn races_by_date(&self, date: NaiveDate) -> Result<Vec<Race>> { ... }
}

// interactor/race/race_odds.rs
impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    pub async fn race_odds(&self, race_id: &RaceId) -> Result<Option<RaceOdds>> { ... }
}
```

`predict_race`（確率推定）は既存メソッドをそのまま再利用する。

---

## オッズ未取得時の動作

`find_race_odds` が `None` を返した場合、EV を計算できず買い目推奨を生成できない。  
このため `select_bets` は呼ばず、以下のフローとする:

1. 「オッズ未取得 — このレースはスキップします」を表示する
2. **`[s]`（スキップ）のみ**を受け付ける（`y` / `e` は提示しない）
3. 賭け金 ¥0 で次のレースへ進む

> 推奨が空の状態で `y` / `e` を提示すると「買えるのに買えない」混乱を招くため、オッズ未取得レースは選択肢をスキップのみに限定する。

---

## Kelly 値の表示と推奨額の算出

`BettingRecommendation.kelly_fraction` は 0.0〜1.0 の小数。表示時は `kelly_fraction * 100` で百分率に変換し `Kelly=15%` のように表示する。

推奨額は以下の手順で算出する（**比例縮小方式**）:

1. 各買い目の素の推奨額を `raw_i = floor(budget * kelly_fraction_i)` で求める
2. `Σ raw_i ≤ budget` ならそのまま推奨額とする
3. `Σ raw_i > budget` の場合、合計が残高に収まるよう全推奨額を比例スケールする  
   `推奨額_i = floor(budget * kelly_fraction_i * (budget / Σ raw_i))`

`kelly_cap = 0.25` のため買い目が 4 本以上あると `Σ kelly_fraction` が 1.0 を超えうる。比例縮小により Kelly の相対比率を保ったまま推奨額合計を残高以内に収め、`y` 選択が残高ガードで弾かれ続ける事態を防ぐ。

---

## ADR

[ADR-0004](../adr/0004-predict-session-binary.md)
