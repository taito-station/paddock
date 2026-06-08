# ADR 0006: 予想精度のバックテスト/評価基盤 (Issue #30)

## ステータス
提案中

## コンテキスト
確率推定 (`paddock_domain::prediction`) や買い目選択 (EV/Kelly, ADR 0003/0005) を変更しても、
その良し悪しを定量比較する手段が無い。過去の `races`/`results` に対して予想ロジックを再現し、
予測と実着順を突合して的中率・回収率・キャリブレーション指標を算出する**バックテスト基盤**を
追加する。これは特徴量拡充 (#31)・品質改善 (#32) の before/after 比較の土台であり、予想ロジック
強化トラックの最優先と位置づける。

### 核心的な課題: データリーク

現状の `horse_stats` / `course_stats` / `jockey_stats` (rdb-gateway) は**日付フィルタ無しで全
`results` を集計**する。レース日 D のレースを評価する際、D 当日・D 以降の結果まで統計に混入すると、
「未来の情報で過去を予測する」データリークになり、評価が過大になる。

評価のために検討した選択肢:

- **案A（as-of 日付カットオフ・walk-forward）**: 各評価レースについて「レース日 D より厳密に前
  (`races.date < D`) の成績のみ」で統計を再計算する。レート集計モデルは非パラメトリックで別途の
  学習フェーズを持たないため、リーク防止 = 統計の as-of カットオフで成立する。
- **案B（固定の train/test 期間分割）**: 期間を train/test に二分し、train 期間の統計で test 期間を
  予測する。実装は単純だが、test 期間の後半レースは古い統計しか使えず、本番の予想 (常に直近まで
  の統計を使う) と条件が乖離する。

オッズ再現について: ADR 0005 で `race_odds` の DB 永続化は撤去済みのため、過去のオッズはスクレイパー
では再現できない。一方 `results.odds`（成績取り込み時に記録された確定オッズ）はテーブルに存在する。

## 決定
**案A（as-of 日付カットオフ・walk-forward）を採用する。**

1. **既存 stats メソッドに `as_of: Option<NaiveDate>` を通す単一コードパス方式**を取る。
   `Repository::horse_stats` / `course_stats` / `jockey_stats` に `as_of` 引数を追加し、
   - `Some(d)` のとき各集計 SQL に `races.date < $d` を付与する（D 当日も除外し未来リークを断つ）。
     `races` を JOIN していない `FROM results` 単独のクエリ（horse の overall / popularity / 枠順、
     jockey の overall / 枠順）は `INNER JOIN races` を足して日付で絞る。`by_surface` / `by_distance_band` /
     course 枠順は既に `races` を JOIN 済み。`as_of` を 1 メソッドに通す単一コードパスを保つため、その
     メソッドが返す全サブ統計に一貫してカットオフを掛ける。
   - 本番 predict (`predict_race`) と analyze の horse/course/jockey コマンドは `None` を渡し、
     従来どおり全期間集計のまま（後方互換・コードパス重複なし）。
2. **過去レース取得用に `Repository::find_finished_races_between(from, to)` を新設**する。
   `source='pdf'` かつ `finishing_position` を持つ確定済みレースを results 付きで返す。
   出馬表 (`find_race_card`) ではなく `results` から `HorseEntry` を復元するため、出馬表が
   無い過去レースもバックテストできる。
3. **指標計算は domain の純粋ロジック `paddock_domain::backtest` に置く**。IO を持たず、
   予測 (`HorseProbability`) と実着順から指標を計算する純粋関数として単体テスト可能にする。
4. **オーケストレーションは `interactor::race::backtest`** に置く。期間内レースを取得し、
   各レースで `as_of=Some(race.date)` の factors を組んで `estimate_probabilities` を再現し、
   実着順と突合する。factors 構築 (`build_factors`) は `predict.rs` と共有する。
5. **CLI は `analyze backtest --from YYYY-MM-DD --to YYYY-MM-DD`** で実行する。
6. **回収率は `results.odds`** を用いる。オッズ欠落レースは回収率の母数から除外し、その他の指標
   （的中率・Brier・LogLoss）は算出する。

### 指標

| 指標 | 定義 |
|-----|-----|
| 単勝的中率 | (win_prob 最大の馬が 1 着のレース数) / 評価レース数 |
| 連対的中率 | 同馬が 2 着以内のレース数 / 評価レース数 |
| 複勝的中率 | 同馬が 3 着以内のレース数 / 評価レース数 |
| 想定回収率 | Σ payout / Σ stake。各レース 100 円を win_prob 最大馬の単勝に賭け、1 着なら `payout = odds×100`、他は 0。`results.odds` が取れるレースのみ母数 |
| Brier (win) | mean((win_prob − y)²)、y=1 if 1 着。全馬エントリ単位 |
| LogLoss (win) | −mean(y·ln p + (1−y)·ln(1−p))。p は `[ε, 1−ε]`（ε=1e-15）にクランプして ln(0) を回避 |

> Brier/LogLoss は、レース内 Σ=1.0 に正規化された `win_prob`（各馬が 1 着になる周辺確率）の較正を、
> 各馬の単勝的中を独立な二値事象とみなして全馬エントリ単位で測る。レース全体の同時分布に対する
> 多クラス LogLoss（`−ln p_winner`）ではない。#31/#32 の before/after を同一定義で一貫比較できれば
> 足りるため、解釈の容易な二値較正を採る。詳細・限界は設計書「指標」「既知の制約」を参照。

## 理由
- 案A は本番予想（常に直近までの統計を使う）と評価条件が一致し、各レースで「その時点で得られた
  情報のみ」を使う walk-forward により評価のリアリズムが高い。案B の固定分割は実装こそ単純だが、
  本番と乖離した条件で測ってしまう。
- `as_of: Option` を既存メソッドに通す方式は、`*_as_of` を別実装するより SQL 重複が無く、本番側は
  `None` で完全後方互換。リーク防止という横断関心を 1 箇所（日付述語）に閉じ込められる。
- 指標を domain の純粋関数に置くことで、DB を伴わず既知入力で期待値を単体テストでき、#31/#32 の
  before/after 比較に安定して使える。
- オッズは `results.odds` を使うことで、撤去済みの `race_odds` 永続化（ADR 0005 案B）を蒸し返さずに
  回収率を概算できる。

## 影響
- `Repository` トレイトの stats 3 メソッドのシグネチャが変わり、全 impl（rdb-gateway）と全呼び出し側
  （predict / horse / course / jockey interactor）に `as_of` 引数の追加・`None` 受け渡しが波及する。
- `results.odds` が未取り込みのレースが多い場合、回収率の母数が小さくなる（的中率・キャリブレーション
  指標は全評価レースで算出可能）。設計書「既知の制約」に明記する。
- バックテストは「statsは全 `results` を横断集計する」前提に乗るため、netkeiba 由来の近走
  (`source='netkeiba'`、過去日付の合成レース) も as-of 統計に含まれる。評価対象レース自体は
  `find_finished_races_between` が `source='pdf'` で絞る。
- 単調性 (`win ≤ place ≤ show`) 非保証や騎手なしペナルティ等、確率推定側の既知制約 (ADR 0002) は
  バックテストの結果にもそのまま現れる。バックテストはそれらの改善 (#32) の効果測定に使う。

## 関連
- ADR 0002（着順確率推定モデル, #11）— 評価対象のロジック
- ADR 0003（EV/Kelly 買い目選択, #12）/ ADR 0005（オッズ結線, #25）— 将来の回収率評価対象
- 設計書 `docs/specifications/backtest.md`
- 設計書 `docs/specifications/probability-estimation.md`
