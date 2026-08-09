# 0061. 脚質（先行度）factor は winner-picking に効かず（棄却）

## ステータス

棄却（#329 脚質/ペース素性の marginal-lift 測定 arc・measure-first）。Phase 1（PR #332・merged）で入れた導出ロジック・dump 列・`horse_past_runs.field_size` 取込は **production 重み 0（`RUNNING_STYLE_WEIGHT=0.0`）で挙動不変のまま dormant 保持**する（jockey_recent_form の ADR 0038 と同型。将来の符号見直し時に再利用可能）。Phase 2 以降の本番統合（Phase 4）には進まない。血統 0058・市場較正 0059・jockey_recent_form 0038 に続く「現行データ天井」の再確認。

## コンテキスト

純モデル resolution の残る唯一の未測定レバーとして「脚質/ペース」＝近走のコーナー通過順位（`horse_past_runs.corner_positions`・#331 Phase0 で取込）から導く先行度シグナルを測った。物差しは AUC/top1/Brier（ROI でない・ADR 0055）。

**最小形（measure-first）**: 先頭コーナー通過順位を出走頭数で相対化した絶対的な先行度スカラー `[0,1]`（1=逃げ・0=追込、`rel=(pos-1)/(field_size-1)`, 先行度=`1-rel`）。近走平均を単一符号「先行度高＝有利」で score に乗せ、この符号が効くかを測定対象にした。within-race 相対化・ペース適性の高度化は「最小形が効いた場合のみ次段」と先送り（プラン eager-spinning-clarke.md）。

血統・クラス arc と同じく **効かない公算が高い前提**（既存 factor に冗長・α=0.2 市場ブレンドに吸収され消える公算大）。よってサンプル 2 段ゲートで scrape コストを抑える measure-first で進めた。

## 決定

脚質（先行度）factor を**本番採用しない**。純モデルで AUC/校正は微改善するが、**本 PJ の本命指標 top1（勝ち馬を当てる精度）が全 weight で劣化**し、単一符号仮説が winner-picking に効かないことが確定した。~3h の全量 scrape をかけて全量確認する価値はない（cheap screen が目的指標の劣化を示したため撤退＝measure-first の狙いどおり）。

## 検証（measure-first・cheap screen で撤退）

**データ経路の整備（Phase 1〜2a）**:
- Phase 1（#332 merged）: `parse_corner_positions`/`leading_position`/`running_style_of_run`（domain）、`HorseFactors.running_style`＋`RUNNING_STYLE_WEIGHT=0.0`＋sweep override（`EstimationConfig.running_style_weight`）、`horse_past_runs.field_size`(BIGINT) の migration/parser/upsert、`RecentRun` 拡張、dump 37 列化＋Python 鏡映。すべて重み 0 で挙動不変。
- Phase 2a: field_size migration を共有 DB に適用。既存行の field_size を `results` の 1 レース頭数 COUNT から backfill（案B・50.4%＝2025-26 の pdf 掲載分）。
- **配管の落とし穴を発見**: `find_recent_runs` は同一実レースを **pdf 優先で dedup** し、pdf 枝は netkeiba 専用列（corner/field_size）を NULL で返す。予測が使う近走は大半が直近＝pdf 掲載レースなので、**corner（#331・表では 99.6%）も field_size もパイプラインに届かず running_style が事実上機能しない**。`(race_id, horse_name)` で twin から carry する修正を試作（未 merge・running_style 棄却により abandon）。ただし carry しても netkeiba 履歴が取得済みの馬（＝過去に予測対象にした馬）に限られ、backtest 窓の馬の **18.0% しか履歴を持たない**ため coverage は ~17% で頭打ち（残り 82% は履歴未取得＝要 scrape）。

**cheap screen（無 scrape）**: 18ヶ月 dump（2025-01-01〜2026-06-30・68,148 行・4,891 レース）の covered subset（running_style 非空 17.3%＝11,809 馬・3,827 レース）で weight sweep（純 α=1.0・忠実性アンカー max|Δ|=8.3e-17）。covered 馬を含むレースを母数に、covered 馬（＝特徴量が効く母数）で per-horse 指標を測定。

| weight | top1 | Δtop1 | AUC | ΔAUC | Brier | ΔBrier | LogLoss | ΔLL |
|---|---|---|---|---|---|---|---|---|
| 0.00 (baseline) | 0.1683 | — | 0.6517 | — | 0.07339 | — | 0.27299 | — |
| 0.10 | 0.1664 | **−0.0018** | 0.6594 | +0.0078 | 0.07323 | −0.00016 | 0.27207 | −0.00092 |
| 0.25 | 0.1651 | **−0.0031** | 0.6662 | +0.0146 | 0.07308 | −0.00030 | 0.27136 | −0.00162 |
| 0.50 | 0.1578 | **−0.0105** | 0.6690 | +0.0173 | 0.07304 | −0.00035 | 0.27136 | −0.00163 |
| 1.00 | 0.1346 | **−0.0337** | 0.6630 | +0.0113 | 0.07343 | +0.00005 | 0.27352 | +0.00053 |

- **top1 は全 weight で単調劣化**（−0.0018〜−0.0337）。AUC・Brier・LogLoss は w=0.1〜0.5 で微改善（AUC 最大 +0.017）。
- 「中位の並びは改善するが勝ち馬の特定はむしろ悪化」＝プランが警告した**単一符号仮説（先行度高＝有利）が winner-picking に効かない**の裏付け。ペースバイアスがコース/距離で反転して絶対スカラーで打ち消す。

## 理由

- **本命指標は top1**（本 PJ は勝ち馬を当てて EV を出す）。中位 AUC 改善は買い目 EV に効かず、しかも純で top1 が劣化する以上 α=0.2 ブレンドで吸収され production 価値に残らない公算大（改善② ADR 0057 が top1 も +0.015 で採用されたのと対照的）。プラン Phase 3 の本ゲート（純 top1 +0.010〜0.015 以上）に**明確に不合格**。
- **単一符号仮説の限界**。先行度は情報を持つ（AUC ↑）が、勝ち馬の特定には効かず crude すぎる。within-race 相対化やペース適性（field 構成×自馬）に高度化する余地はあるが、最小形が top1 を悪化させた以上、次段に進む前提（プラン「最小形が効いた場合のみ」）を満たさない。
- **coverage 天井と scrape コスト**。全量クリーン測定には backtest 窓の未取得馬（1ヶ月で ~3,068 頭・~2.8h／2ヶ月で ~4,532 頭・~4.1h の JRA scrape）が必要。cheap screen が目的指標の劣化を示した以上、この scrape は arc が回避しようとしていたコストそのもの。
- ADR 0027（データ量は resolution の主レバーでない）・0058（factor 冗長性）と整合。純 resolution の残り gap（純 AUC 0.671 vs 市場 0.833）は新 factor でも詰まらない。

## 留保

- cheap screen は **covered subset（18%・過去に予測対象にした馬に偏り）**なので top1 の絶対水準にバイアスがありうる。ただし top1 の**単調劣化**という向きは頑健で、全量 scrape で反転する可能性は低いと判断（AUC 改善が top1 に結び付かない構造は subset 非依存）。確証が要る場合は Phase 2c 全量 backfill→Phase 3 本ゲートだが、コスト対効果で見送り。

## 影響

- **本番挙動は不変**（`RUNNING_STYLE_WEIGHT=0.0` のまま dormant）。Phase 1（#332）の導出・dump 列・field_size 取込は保持（ADR 0038 と同方針・将来の符号見直し用）。
- **carry 修正（find_recent_runs で corner/field_size を twin から carry）は未 merge で abandon**。corner/field_size は running_style 以外に consumer が無く、dormant 中は配管する価値がない（YAGNI）。#329 を再開する場合はこの carry 修正＋窓内馬の履歴 scrape が前提になる。
- 共有 DB に適用済みの field_size migration（#332）と案B backfill（field_size 部分埋め）は無害な dormant データとして残置。
- 測定スクリプト（`/tmp/pa/rs_sweep.py` 等）は本番外の使い捨て scratch でリポに残さない。再提案防止の記録として本 ADR に集約。
- 関連: 0038（jockey_recent_form 棄却・dormant 保持の先例）/0057（改善②補完・採用）/0058（血統棄却・factor 冗長性）/0059（市場較正棄却）/0027（データ量は主レバーでない）/0055（EV 層分離）。

## 再現

```sh
# 1. 純 dump（18ヶ月・改善②込み production 相当）
./target/release/paddock-analyze backtest --from 2025-01-01 --to 2026-06-30 \
  --blend-alpha 1.0 --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 \
  --dump-features /tmp/pa/pure_long.tsv
# 2. running_style weight sweep（feature_resolution_diag.py の再構成を流用・covered subset）
#    忠実性アンカー max|Δ|=8.3e-17・上表を出力
python3 /tmp/pa/rs_sweep.py /tmp/pa/pure_long.tsv
```
