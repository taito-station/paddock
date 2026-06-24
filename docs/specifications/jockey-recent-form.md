# 騎手直近フォーム特徴量仕様書

Issue #221 対応。現行の `jockey_surface`（騎手の通算芝ダ別勝率）は直近の好不調を捉えられないため、
騎手の直近 N 走フォームスコアを新特徴量として追加する。

## 概要

![騎手直近フォームデータフロー](diagrams/jockey-recent-form-dataflow.svg)

`results`（PDF確定成績）と `horse_past_runs`（netkeiba近走）から当該騎手の直近 N 走を取得し、
「着順 vs 人気乖離」シグナルの平均で [0, 1] のフォームスコアを算出する。
`HorseFactors.jockey_recent_form` として `raw_score` の重み付き平均に加える。
`jockey_surface`（通算率）とは独立した項で、乗り替わり直後の絶好調騎手や不振中の騎手の識別を目的とする。

---

## 背景と課題

| 現行特徴量 | 課題 |
|-----------|------|
| `jockey_surface` | 通算芝ダ別勝率。長期平均のため直近の好不調に反応しない |

馬の `recent_form`（前走フォーム）は直近 1 走の人気乖離・着差・タイム・間隔・体重変化を
複合して [0,1] に写像し、PR #31 で有効性が確認された（ADR 0009）。
騎手版として「直近 N 走の人気乖離平均」を導入する。

---

## 変更範囲

### 1. domain (`paddock_domain`)

#### 1.1 新型: `JockeyFormRun`

```rust
// domain/src/prediction/model.rs に追加
pub struct JockeyFormRun {
    pub finishing_position: Option<u32>,
    pub popularity: Option<u32>,
}
```

着順・人気のみを持つ軽量型。`HorseResult` を流用しないのは、
タイム・体重・着差等の不要フィールドをリポジトリが取得しなくてよいようにするため。

#### 1.2 新フィールド: `HorseFactors.jockey_recent_form`

```rust
// domain/src/prediction/model.rs
pub struct HorseFactors {
    // 既存フィールド略…
    /// 騎手直近 N 走フォームスコア [0,1]（0.5=中立）。
    /// 騎手未登録・N 走分の近走データ無し・有効 signal ゼロは `None`（母数除外）。
    pub jockey_recent_form: Option<f64>,
}
```

#### 1.3 新関数: `jockey_recent_form_score`

```rust
// domain/src/prediction/scoring.rs
pub fn jockey_recent_form_score(runs: &[JockeyFormRun]) -> Option<f64> {
    let signals: Vec<f64> = runs.iter().filter_map(|r| {
        if let (Some(pop), Some(pos)) = (r.popularity, r.finishing_position) {
            let gap = pop as f64 - pos as f64; // >0: 人気以上の好走
            Some((0.5 + gap * POP_GAP_K).clamp(0.0, 1.0))
        } else {
            None
        }
    }).collect();
    if signals.is_empty() { None } else { Some(signals.iter().sum::<f64>() / signals.len() as f64) }
}
```

**signal 設計の根拠:**
- 着順 vs 人気乖離（`POP_GAP_K = 0.08`）は horse の `recent_form_score` でも使用済みの有効 sub-signal
- 馬体重変化・前走間隔・着差・タイムは騎手属性でなく馬・コース属性のため N 走平均に混ぜない
- シンプルなスカラーで jockey_surface との重複寄与を最小化する

#### 1.4 新定数: `JOCKEY_RECENT_FORM_WEIGHT`

```rust
// domain/src/prediction/weights.rs
/// 騎手直近フォーム項の重み（暫定 0.25）。backtest sweep で決定する。
pub(crate) const JOCKEY_RECENT_FORM_WEIGHT: f64 = 0.25;
```

初期値は `FORM_WEIGHT`（馬の前走フォーム）と同値の保守値。
バックテスト結果によって 0.0 に設定し無効化することも有り得る。

---

### 2. use-case repository trait

```rust
// use-case/src/repository.rs
/// JockeyFormRun: 騎手直近フォーム算出用の軽量型（jockey_recent_form_score に渡す）。
pub struct JockeyFormRun {
    pub finishing_position: Option<u32>,
    pub popularity: Option<u32>,
}

trait StatsRepository {
    // 新メソッド
    fn jockey_recent_runs_batch(
        &self,
        jockeys: &[JockeyName],
        before: NaiveDate,
        limit: u32,
    ) -> impl Future<Output = Result<HashMap<JockeyName, Vec<JockeyFormRun>>>> + Send;
}
```

デフォルト実装は per-jockey ループ。rdb-gateway のみウィンドウ関数で一括 override。

---

### 3. rdb-gateway

`find_recent_runs.rs` と同様の UNION dedup クエリを騎手名フィルタで実装する。

```sql
-- 単体版（既定実装から呼ばれる）
WITH unioned AS (
    SELECT races.date, races.venue, races.race_num,
           results.finishing_position, results.popularity,
           0 AS src_rank, results.race_id
    FROM results
    INNER JOIN races ON races.race_id = results.race_id
    WHERE results.jockey = $1 AND races.date < $2 AND races.source = 'pdf'
    UNION ALL
    SELECT date, venue, race_num,
           finishing_position, popularity,
           1 AS src_rank, race_id
    FROM horse_past_runs
    WHERE jockey = $1 AND date < $2
)
SELECT u.finishing_position, u.popularity
FROM unioned u
WHERE NOT EXISTS (
    SELECT 1 FROM unioned u2
    WHERE u2.date = u.date AND u2.venue = u.venue AND u2.race_num = u.race_num
      AND (u2.src_rank < u.src_rank
           OR (u2.src_rank = u.src_rank AND u2.race_id > u.race_id))
)
ORDER BY u.date DESC, u.race_id DESC
LIMIT $3
```

バッチ版は `PARTITION BY jockey ORDER BY date DESC, race_id DESC` で全騎手を一括取得する。

---

### 4. use-case predict / backtest

#### predict.rs

```rust
// try_join! の 5 番目として追加
let jockey_form_map = self.repository
    .jockey_recent_runs_batch(&jockey_names, card.date, JOCKEY_RECENT_FORM_LIMIT)
    .await?;

// build_factors に渡す
let jockey_recent_form = entry.jockey.as_ref()
    .and_then(|j| jockey_form_map.get(j))
    .and_then(|runs| paddock_domain::prediction::jockey_recent_form_score(runs));
```

#### backtest.rs

バックテスト経路でも同様にバッチ取得し `build_factors` に渡す。
スイープパラメータ:

| パラメータ | 値 |
|-----------|-----|
| N（走数上限） | 5 / 10 / 20 |
| JOCKEY_RECENT_FORM_WEIGHT | 0.0 / 0.25 / 0.5 / 1.0 |

`EstimationConfig` または 定数差し替えでスイープを回す（馬の `recent_form` sweep と同パターン）。

---

### 5. 定数: `JOCKEY_RECENT_FORM_LIMIT`

```rust
// use-case/src/interactor/race/predict.rs
const JOCKEY_RECENT_FORM_LIMIT: u32 = 10; // backtest sweep で 5 / 10 / 20 を評価後に確定
```

---

## バックテスト評価方針

### 評価期間

現行と同一: `--from 2026-03-28 --to 2026-05-31`（約 140 レース）

### 評価指標

1. 単勝的中率 / 複勝的中率
2. 単勝回収率 / 複勝回収率（curated 推奨買いベース）
3. Brier score（単勝 / 複勝）
4. LogLoss（単勝）

### 採用基準

- **複数の指標でベースライン（weight=0.0）を上回る場合** → 本番化し ADR に記録
- **改善なし・悪化** → weight=0.0 のまま棄却記録を ADR に残す

`jockey_surface` との交互作用（多重共線性）は Brier / LogLoss の変化量で間接的に観察する。

---

## 関連

- Issue #31（馬版前走フォーム）
- ADR 0009（FORM_WEIGHT 採用・recent_form 有効化）
- ADR 0016（recency 時間減衰棄却）
- ADR 0017（jockey_surface 専用縮約棄却）
- ADR 0034（alpha 再調整・recency 棄却）
