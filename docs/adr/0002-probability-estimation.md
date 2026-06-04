# ADR 0002: 着順確率推定モデルの実装 (Issue #11)

## ステータス
承認済み

## コンテキスト
Issue #11 で、DB に蓄積された過去成績をもとに出走馬ごとの 1 着・2 着・3 着確率を
推定するモデルが求められた。

既存のスタッツ基盤（`horse_stats` / `course_stats` / `jockey_stats`）はすでに実装済みで、
枠順・芝ダ・距離帯・騎手別の勝率・連対率は取得可能である。ただし複勝率（3 着以内）は
保持していない。

Issue 本文に「精密さより動くことを優先」とあり、機械学習ではなくルールベーススコアリングで十分。

## 決定

1. **ルールベーススコアリングを Domain 層に実装する。**  
   `paddock_domain::prediction` モジュールを新設し、`HorseProbability` 型と
   `estimate_probabilities` 純粋関数を置く。IO なし・テスト容易な設計にする。

2. **`GroupStat` に `shows: u32`（複勝カウント）を追加する。**  
   既存の `places`（連対カウント、top-2）に加え `shows`（複勝カウント、top-3）を追加する。
   スキーマ変更ではなく既存クエリへの集計カラム追加で対応する。

3. **スコアリング重みは固定値とする。**  
   `course_gate_rate(×2) + horse_surface_rate(×1) + horse_distance_rate(×1) + jockey_surface_rate(×1)`  
   `course_gate_rate` を 2 倍にする理由: 会場・距離・馬場・枠順の組み合わせは個別レースへの適合度が最も直接的で信頼度が高いため。  
   チューニングより動くことを優先し、パラメータ化は行わない。

4. **`find_race_card(race_id)` を Repository に追加する。**  
   `predict_race` ユースケースは race_id を受け取り DB からエントリを取得する。
   CLI 引数でエントリを逐一渡す方式は操作性が低いため採用しない。

5. **`analyze` アプリに `predict <race_id>` サブコマンドを追加する。**  
   既存の `horse` / `course` / `jockey` サブコマンドと同列に配置する。

## 理由

- 純粋関数として Domain 層に置くことで、ユニットテストが外部依存なしで書ける
- `GroupStat` への `shows` 追加は破壊的変更だが、変更箇所が repository 実装に局所化しており、
  コンパイラが変更漏れを全て検出する
- `find_race_card` は既存の `save_race_card` と対をなす自然な拡張であり、
  リポジトリトレイトのセマンティクスを壊さない

## 影響

- `GroupStat` の全コンストラクタと SQL クエリを更新する必要がある
  （horse_stats × 6 パターン、course_stats × 1 パターン、jockey_stats × 3 パターン）
- `shows` フィールドは `predict` ユースケース以外から使われないため Rust コンパイラの `dead_code` 警告が出る可能性がある。`#[allow(dead_code)]` を付けるか、将来 `print_section` で複勝率を表示する際に解消する
- 複勝率カラムは既存の `print_section` 出力には含めない（stats 表示の変更はスコープ外）
- 確率値はあくまで参考値であり、オッズ等他の情報と組み合わせて使うことを想定する
