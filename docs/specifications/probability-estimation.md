# 着順確率推定モデル仕様書

Issue #11 対応。DB に蓄積された過去成績をもとに、出走馬ごとの 1 着・2 着・3 着確率を推定する。

## 概要

![着順確率推定フロー](diagrams/probability-estimation-flow.svg)

出馬表（`RaceCard`）を受け取り、各馬の勝率（win）・連対率（place）・複勝率（show）の推定確率を返す。
精密な機械学習モデルではなく、「データがあれば動く」ルールベーススコアリングを採用する。

---

## 入力

| 項目 | 型 | 説明 |
|------|----|----|
| `RaceCard` | ドメイン型 | race_id / venue / distance / surface / entries |
| `HorseEntry` (entries 内) | ドメイン型 | gate_num / horse_num / horse_name / jockey (Option) |

## 出力

| 項目 | 型 | 説明 |
|------|----|----|
| `horse_num` | `HorseNum` | 馬番 |
| `horse_name` | `HorseName` | 馬名 |
| `win_prob` | `f64` | 1 着確率（0.0〜1.0） |
| `place_prob` | `f64` | 2 着以内確率（0.0〜1.0） |
| `show_prob` | `f64` | 3 着以内確率（0.0〜1.0） |

レース内で各確率の合計が 1.0 になるよう正規化する。

---

## スコアリングアルゴリズム

### ステップ 1: 統計データの取得

各 `HorseEntry` に対して以下 3 種のスタッツをDBから並列取得する。

| スタッツ | キー | 取得データ |
|---------|-----|----------|
| `course_stats` | venue × distance × surface | 枠順グループ別 勝率 / 連対率 / 複勝率 |
| `horse_stats` | horse_name | 芝ダ別・距離帯別・枠順グループ別 勝率 / 連対率 / 複勝率 |
| `jockey_stats` | jockey_name (任意) | 芝ダ別 勝率 / 連対率 / 複勝率 |

### ステップ 2: 生スコア計算

馬ごとに以下の加重和を計算する（勝率・連対率・複勝率それぞれ）。

```
raw_score =
    2.0 × course_gate_rate          // コース×枠順（最も信頼度高）
  + 1.0 × horse_surface_rate        // 馬の芝ダ実績
  + 1.0 × horse_distance_rate       // 馬の距離帯実績
  + 1.0 × jockey_surface_rate       // 騎手の芝ダ実績（騎手なし時は 0）
```

合計重み = 5.0（騎手あり）または 4.0（騎手なし）

### ステップ 3: レース内正規化

全出走馬の生スコアの合計で各馬スコアを割る。

```
win_prob_i = raw_win_score_i / Σ(raw_win_score_j)
```

スコア合計が 0 の場合（統計データが全馬 0 件）は、均等確率（1/頭数）にフォールバックする。

---

## 統計データ拡張: GroupStat への `shows` 追加

現行の `GroupStat` は 連対（1〜2 着）までしか保持しない。複勝率（1〜3 着）を扱うため `shows` フィールドを追加する。

```rust
pub struct GroupStat {
    pub label: String,
    pub starts: u32,
    pub wins: u32,
    pub places: u32,  // 連対 (top-2)
    pub shows: u32,   // 複勝 (top-3) ← 追加
}
```

DBクエリは以下を追加:
```sql
SUM(CASE WHEN finishing_position IN (1,2,3) THEN 1 ELSE 0 END) AS shows
```

---

## レイヤー別実装方針

### Domain (`paddock_domain::prediction`)

```rust
pub struct HorseProbability {
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    pub win_prob: f64,
    pub place_prob: f64,
    pub show_prob: f64,
}

pub fn estimate_probabilities(
    entries: &[(HorseEntry, HorseFactors)],
) -> Vec<HorseProbability>
```

`HorseFactors` は horse_stats / course_stats / jockey_stats（Option）を束ねる中間型。  
スコアリングと正規化の純粋関数として実装（IO なし・テスト容易）。

### Use-Case (`use_case::interactor::race::predict`)

```rust
pub async fn predict_race(&self, race_id: &RaceId) -> Result<Vec<HorseProbability>>
```

1. `find_race_card(race_id)` → RaceCard 取得
2. 各 HorseEntry に対してスタッツを並列取得
3. `domain::prediction::estimate_probabilities` を呼ぶ

Repository に `find_race_card` メソッドを追加する。

### Interface (rdb-gateway)

- `find_race_card` SQL: race_cards / race_card_entries テーブルから取得
- 既存の horse_stats / course_stats / jockey_stats クエリに `shows` カラムを追加

### Apps (analyze)

```
paddock-analyze predict <race_id>
```

出力例：
```
# レース予測 2026060412R02
馬番  馬名            勝率     連対率   複勝率
  1  ガリレオトライ  18.3%   36.7%   55.1%
  2  テスラブルー    12.1%   24.2%   36.3%
  ...
```

---

## 既知の制約

- スタッツ件数が少ない馬（デビュー直後等）は win_rate = 0 になり、均等フォールバックに入ることがある
- コースデータが存在しない組み合わせ（venue × distance × surface）の場合も均等フォールバック
- モデルは過去成績のみを使用。馬場状態・前走間隔・調教等は考慮しない
- 確率の絶対値より**レース内の相対的な傾向**を見るための参考値として使うこと
