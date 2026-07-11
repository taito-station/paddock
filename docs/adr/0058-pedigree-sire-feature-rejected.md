# 0058. 血統（種牡馬）適性 factor は現行データの天井内でノイズ級（棄却）

## ステータス

棄却（#272 純モデル resolution 改善 arc・新データソース取得 arc）。本番コードは変更なし（measure-first ゲートで撤退したため配管ゼロ）。改善①（ADR 0056）＋改善②（ADR 0057）で到達した純 top1 0.162→0.197・AUC 0.649→0.678 は merged 済みで不変。

## 訂正（2026-07-02・追測で判明）

本 ADR 初版の「構造的天井は coverage（sire を乗せられるのは 19.5%）」という論拠は**誤診**だった。棄却の verdict（sire はノイズ級・不採用）は変わらないが、根拠を factor 冗長性に訂正する:

- **19.5% は sire 固有のアーティファクト**。sire を dump 行へ join する際、`results.horse_id`（backfill が弱く 20.6%）／`horses` 名前引き（同 20.6%・pedigree を 2124 頭しか fetch していない）を使ったため。**馬 factor 一般の天井ではない。**
- **馬履歴 factor の実 coverage は ~60-71%**。backtest は `horse_surface`/`horse_distance`/`horse_track_condition` を **`results` の過去成績の名前引き**で作る（horse_id 不要・2017-2026 の全成績が母数）。`course_gate`(95.8%) は `course.by_gate_group`＝コース×枠の汎用バイアスで馬履歴不要。
- **coverage を上げても resolution は改善しない**（`scripts/predict-check/coverage_strata.py --tsv <pure dump>`・gated 4,594R を「1レース内の馬履歴 factor カバー率」で層別）:

| horse-history coverage | races | AUC_model | AUC_market |
|---|---:|---:|---:|
| 0% | 437 | 0.677 | 0.776 |
| 0-25% | 92 | 0.610 | 0.823 |
| 25-50% | 169 | 0.660 | 0.863 |
| 50-75% | 482 | 0.651 | 0.860 |
| 75-99% | 1,649 | 0.664 | 0.845 |
| 100% | 1,765 | 0.685 | 0.824 |

model AUC は coverage 層でフラット（0.61-0.685・上に単調増でない）。**フル装備の 100% 層(0.685) は履歴ゼロの 0% 層(0.677) をわずか +0.008 しか上回らない**＝馬履歴 factor は常在の course_gate/jockey/trainer に**冗長**。よって天井は **coverage でなく factor 冗長性**（ADR 0027〔データ量は主レバーでない〕・ADR 0057 の drop 下 ablation〔馬 factor 除去でも top1 ほぼ不変〕と整合）。全 runner 履歴の大量 fetch arc も、sire を高 coverage で再測定することも、この冗長性ゆえ不要。

※ caveat: 層はレース母集団が非同質（0% 層は新馬等に偏りうる・AUC_market も 0.78-0.86 と層で振れる）ため、端点比較は coverage 効果と race-type 交絡込みの directional read。ただし **0%↔100% 層の +0.008 は欠損補完が無関係な端点なので drop/impute どちらのモデルでも成立**（endpoint はモデル非依存）＝この load-bearing 比較は交絡・補完方式に頑健。ADR 0057 の ablation（drop 下で馬 factor 除去しても top1 ほぼ不変）も同方向の支持。

## コンテキスト

既存 netkeiba データで測れる resolution レバーは測り尽くした（重み空間は ADR 0056 で最良化・within-race z-score は同 0056 で悪化確認・recency は ADR 0034 で棄却・クラス昇降は `class_prototype` で撤退）。ADR 0027（精度の主レバー＝市場ブレンドでデータ量でない）と整合し、純 AUC 0.678 vs 市場 0.833 の残り gap は**現行データでは構造的**と判断していた。

唯一残る伸び代として「**全く新しいデータソース**」を取得する arc に踏み込んだ。ターゲットは**血統（種牡馬 sire）**。選定根拠: 構造化・fetchable、factor 形式が明快（種牡馬×surface/距離の産駒成績率）、既存 factor と直交しうる（自馬実績が薄い若馬で種牡馬適性が効く＝改善②の弱点補完）。

クラス arc の教訓（pre-gate POSITIVE でも marginal-lift 不合格で撤退＝本番配管が無駄になった）を踏まえ、**measure-first**（使い捨てサンプル取得→Python で as-of marginal-lift を測定→効けば本番 build、効かねば配管ゼロで撤退）で進めた。物差しは **Brier/AUC/top1（ROI でない, ADR 0055）**。

## 決定

血統（種牡馬）適性 factor を**採用しない**。as-of 自前集計は現行データの天井内でノイズ級 lift しか出さず、本番配管（parser/schema/backfill/factor 統合）を作る価値がない（天井の性質は上記「訂正」＝factor 冗長性）。

## 検証（measure-first ゲート）

**データ取得**: 全 2124 頭の netkeiba 血統ページ（`db.netkeiba.com/horse/ped/{id}/` の `blood_table` 先頭 td＝種牡馬）を使い捨てスクリプトで fetch（失敗 0・sire 100%）。distinct sires=266・median 2 progeny/sire・110 sires は産駒 1 頭のみ。

**as-of 種牡馬適性**: 自 DB 産駒 `horse_past_runs` から対象レース日より前・自馬除外の産駒成績を m=10 縮約で集計（リーク無し・in-house）。overall／surface／distance／both（surface∩距離）× 重み {0.5,1.0,2.0} を pure dump に join して純 AUC/top1/Brier を測定（`pedigree_prototype.py`・忠実性 1.11e-16）。

| 構成（純 α=1.0, gated 4,594R, baseline=drop） | AUC | top1 | Brier |
|---|---|---|---|
| baseline（既存 6 factor・drop） | 0.6708 | 0.1824 | 0.0655 |
| +sire overall  w=1.0 | 0.6719 (+0.0011) | 0.1842 (+0.0017) | ±0 |
| +sire overall  w=2.0 | 0.6719 (+0.0010) | 0.1844 (+0.0020) | ±0 |
| +sire surface  w=1.0 | 0.6717 (+0.0009) | 0.1839 (+0.0015) | ±0 |
| +sire distance w=1.0 | 0.6712 (+0.0003) | 0.1842 (+0.0017) | ±0 |
| +sire both     w=2.0 | 0.6697 (−0.0012) | 0.1805 (−0.0020) | ±0 |

※ 上表は測定した全 12 構成（overall／surface／distance／both × 重み {0.5,1.0,2.0}）からの抜粋で、各指標の最良行（overall w=1.0/2.0）・本文で言及する surface/distance の代表行（各 w=1.0）・最悪行（both w=2.0）を提示したもの。下記「各指標の全構成最大」は 12 構成すべてに対する最大値。Δ は表示 4 桁でなくフル精度の baseline との差から算出（同一表示値でも Δ が僅かに異なるのはこのため）。Brier は全構成で |Δ|<0.00005 のため表示上 ±0（both の劣化は AUC/top1 に表れる）。baseline=drop は改善①相当で改善②の impute は未反映（ステータス掲載の merged 値 0.678/0.197 とは別物）。

- **各指標の全構成最大**でも AUC +0.0011（overall w=1.0）・top1 +0.0020（overall w=2.0）・Brier ±0＝単一構成が両指標を同時達成するわけではない。surface モードは AUC +0.0009 で overall に届かない。改善①（AUC +0.022）比で約 20 倍・桁違いに小さく、改善②（+0.007）比でも約 1/6、**棄却済みクラス arc（top1 最良 +0.0015「ノイズ級」）とほぼ同水準**の実務上ノイズ。棄却は有意性検定でなく、この絶対水準の小ささと上記「訂正」の factor 冗長性で判断する（top1 の周辺 SE ≈0.0057 は対応差の SE でなく粗い上界にすぎず、有意/非有意の物差しには使わない）。
- 「both」は surface∩距離で過スパースになり有害。high weight も AUC を削る＝positive は脆い。

## 理由

- **天井は factor 冗長性**（上記「訂正」参照）。直接の馬能力 factor（horse_surface/distance/track_condition, 実 coverage ~60-71%）ですら、それが常在の course_gate/jockey/trainer に対し full 装備レースで +0.008 AUC しか足せない（coverage 層別）。種牡馬適性はその馬能力のさらに弱い代理なので、乗せる層を広げても full-field 指標の上振れ余地は小さい。median 2 progeny/sire の母数薄は二次要因。
  - なお sire を dump に乗せられたのは 19.5% だが、これは**馬 factor 一般の天井でなく pedigree を 2124 頭しか fetch していない sire 固有の制約**（初版はこれを coverage cap と誤診・上記訂正）。
- **baseline は改善①(drop) で測った**（Python ミラーが改善②の impute 未実装のため）。impute は既存欠落 factor を field mean で埋めるので sire の marginal 余地はむしろ縮むと見込まれる（directional な想定・未計測で、sire×impute の交互作用が単調である保証はない）。ただし**棄却の主根拠は a fortiori でなく上記 factor 冗長性**であり、baseline の drop/impute 差はその結論を揺るがさない。
- ADR 0027（データ量は resolution の主レバーでない）を、クラスに続き血統でも再確認。純 resolution の残り gap は「新 factor 追加」でも「coverage 拡大」でも詰まらない。

## スコープ外 / 次にありうる伸び代

- **全 runner 履歴の大量 fetch（数万頭・coverage 拡大）arc は測定して否定済み**（上記「訂正」の coverage 層別＝100% 層でも +0.008 AUC）。初版は「coverage cap を上げれば動く」と書いたが誤り。次に resolution を動かすなら coverage でも新 factor でもなく、公開データ外の情報が要る（ADR 0027）。
- netkeiba 既成 sire 集計（厚い母数）の scrape は fallback として検討したが、sire を乗せられる層が pedigree fetch 範囲に縛られ、かつ既成集計は as-of でない（リーク）ため見送り。仮に厚くしても factor 冗長性ゆえ効かない。
- 本 marginal-lift は改善①(drop) baseline 上で測っており、本番 merged（改善② impute 込み）baseline での再測定はしていない（impute は sire の余地をむしろ縮める見込みで、結論は factor 冗長性で立つ）。将来 pedigree を再検討する場合もこの限界を踏まえること。
- 学習モデル（ADR 0053 棄却）・isotonic（#319 診断で棄却）には戻らない。

## 補記（2026-07-10）: full-field 血統再測定は不要（precedent）

将来「本 arc は 2124 頭サンプルにすぎない。本番窓の全馬（gated 4,594R の延べ ~68,000 出走・distinct horse 数万規模）で血統を高 coverage に fetch し直せば効くのでは」という**同種提案が再燃した場合に一蹴するため**、何が測定済みで何が未実施かを明示し、full-field 再測定 arc を走らせない判断を precedent として残す。

**未実施（正直に記す）**:

1. 血統(sire)を ~70% coverage まで（pedigree を数千〜数万頭追加 fetch）拡大した**直接**再測定。本 arc は 2124 頭・sire を dump へ乗せられたのは 19.5% 層に留まる（＝pedigree 2124 頭 fetch 由来の sire 固有制約であって馬 factor 一般の coverage 天井ではない・上記訂正）。
2. 本番 merged baseline（改善② impute 込み・純 top1 0.197・AUC 0.678）上での血統 marginal-lift 再測定。本 arc は改善①(drop) baseline 上で測っている。

**それでも実行しない根拠**（precedent の核）:

- **coverage 拡大の無効性は endpoint で証明済み・モデル非依存**。上記「訂正」の coverage 層別（`scripts/predict-check/coverage_strata.py`）が、馬履歴 factor の 100% 装備層(AUC 0.685)が履歴ゼロの 0% 層(0.677)を **+0.008 しか上回らない**ことを示す。この端点比較は欠損補完が無関係な 0%↔100% で成立するため **drop/impute どちらのモデルでも頑健**（未実施項目 2 の baseline 差に影響されない）。
- **血統は馬履歴 factor のさらに弱い代理**（自馬実績が薄い若馬でのみ種牡馬適性が効く設計）。直接の馬能力 factor（horse_surface/distance/track_condition・実 coverage ~60-71%）ですら full 装備で +0.008 AUC しか足せないのだから、その弱い代理を full coverage へ広げても full-field 指標の上振れ余地は **端点 +0.008 を上界に頭打ち**（実証済み endpoint に基づく a fortiori・棄却の主根拠はあくまで上記 factor 冗長性）。実測でも sire 最良は AUC +0.0011・top1 +0.0020 で、この上界の内側にノイズ級で収まっている。
- **impute baseline では余地はむしろ縮む**。既存欠落 factor が field mean で埋まる分、sire の marginal 余地は drop baseline より小さくなる見込み（directional な想定で sire×impute の単調性は未保証だが、測っても drop の +0.0020 が上界の見込み）。
- したがって数千〜数万頭の追加 fetch（`db.netkeiba.com/horse/ped/` を pacing 3s で数時間〜）の配管・運用コストに対し、リターンは endpoint 上界（+0.008 AUC・実測ノイズ級 +0.0020 top1）で頭打ち。measure-first の撤退基準（本番配管を作る価値がない）を **full-field でも満たす**。

**再検討を認める唯一の条件**: 本 arc が測ったのは「as-of 種牡馬(sire)単独の overall/surface/距離適性」に限る。**母父(dam sire)・インブリード(近交係数)・ニックス**等は ADR 未測定であり、これらは sire 単独と直交する**新シグナル**でありうる（単なる coverage 拡大・母数増ではない）。将来 pedigree を再検討するなら、この直交角度を伴う場合に限る。**「全馬で sire を測り直す」型の coverage/母数拡大は本節で却下する**（factor 冗長性ゆえ結論不変）。

## 影響

- 本番コード・スキーマ・CLAUDE.md いずれも変更なし。ADR と本 arc の測定記録のみ。
- 測定スクリプト（`fetch_pedigree.py`/`pedigree_prototype.py`）は本番外の使い捨て（scratch `/tmp/pa/`）。再提案防止の記録として本 ADR に集約。
- 関連: 0027（精度の主レバー＝市場ブレンド）/0034（recency 棄却）/0055（EV 層分離・純モデル化）/0056（改善①重み・within-race 悪化）/0057（改善②補完）/0053（学習モデル棄却）。純モデル resolution arc の到達点は「現行データ天井」で確定。

## 再現

`fetch_pedigree.py`/`pedigree_prototype.py` と入力（`horse_ids.txt`・`runner_hid`/`race_meta`/`progeny_runs` の DB エクスポート）は本番外の使い捨て scratch でリポには残さない。以下は測定を再走させるための手順記録であり、忠実性 1.11e-16 と上表の出所を示す目的（リポ単独で完全再現する成果物ではない）。

```sh
# 1. 純 dump（改善①相当・drop baseline）
./target/release/paddock-analyze backtest --from 2025-01-01 --to 2026-06-30 \
  --blend-alpha 1.0 --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 \
  --dump-features /tmp/pa/pure.tsv
# 2. 全馬の種牡馬を fetch（db.netkeiba.com/horse/ped/{id}/・EUC-JP・pacing 3s）
python3 /tmp/pa/fetch_pedigree.py /tmp/pa/horse_ids.txt /tmp/pa/pedigree.tsv 3.0
# 3. as-of marginal-lift ゲート（runner_hid/race_meta/progeny_runs は DB からエクスポート）
python3 /tmp/pa/pedigree_prototype.py   # 忠実性 1.11e-16・上表を出力
```
