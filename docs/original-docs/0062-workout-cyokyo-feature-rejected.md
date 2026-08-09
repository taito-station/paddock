# 0062. 調教（追い切り）評価 factor は市場ブレンドに吸収され不採用（棄却）

## ステータス

棄却（#327 調教データの marginal-lift 測定 arc・measure-first）。本番コード変更なし（配管ゼロで撤退）。血統 0058・市場較正 0059・脚質 0061 に続き、**「現行＋取得可能な公開新データ（血統/脚質/調教）は全て市場ブレンド(α=0.2)に吸収される」**を確定＝純モデル resolution 路線は完全に closed（ADR 0027）。次の伸び代は執行エッジ（ADR 0055/0060）のみ。

## コンテキスト

純モデル resolution の残る唯一の未探索 fundamental レバーが調教（追い切り）（#327）。issue は JV-Link（Windows COM SDK）の生タイムを想定していたが、取得経路の確立が非自明（この PJ は macOS/Lima 中心）。

調査の結果、**netkeiba 追い切りページ `race.netkeiba.com/race/oikiri.html?race_id=<12桁>` が無料で「調教評価(A〜D)＋短評」を HTML テーブルで提供**（全出走馬・archived で as-of 安全・1レース1fetch で激安）と判明。JV-Link を回避し、既存 netkeiba scraper と同じ経路で cheap screen できる。粗い主観グレードだが「これが効かないなら生タイム（premium/JV-VAN）も効かぬ公算が高く安く切れる」との判断で、**最小コストの netkeiba 無料 A〜D 評価から measure-first**（血統 arc ADR 0058 と同型・scratch-first）。物差しは AUC/top1/Brier（ROI でない・ADR 0055）。

## 決定

調教評価 factor を**本番採用しない**。純モデルでは明確な resolution 情報を持つ（血統/脚質より良い）が、**本番の市場ブレンド(α=0.2)で完全に吸収される**（市場が調教評価を既にオッズへ織り込む・ADR 0027）。粗い A〜D すら吸収される以上、高コストな JV-VAN 生タイムも吸収される公算が極めて高く、調教方向（公開データ）全体を見送る。

## 検証（measure-first・cheap screen で撤退）

**データ取得（Phase 1・scratch `/tmp/pa/fetch_cyokyo.py`）**: 2026-04〜05 の gated 593 レースの oikiri ページを scrape（pacing 3s・~30分）。canonical→netkeiba 変換は `Venue::as_code`（`src/use-case/src/netkeiba_race_id.rs`）を写経。**coverage 99.2%**（全 593 レースで table 取得・8395 行中 8331 grade 付）。グレード分布 B 58%/C 36%/D 4%/A 1.2%（B/C 偏りだが 4 段・縮退なし）。JV-Link 不要。

**marginal-lift（Phase 2・`/tmp/pa/cyokyo_prototype.py`・`feature_resolution_diag.py` 鏡映・忠実性 5.55e-17）**: 調教評価を ordinal scalar（A=1.0/B=2⁄3/C=1⁄3/D=0.0・欠落は母数除外）に encode し純 dump（2025-01〜2026-06 の窓 593R・8304 covered 馬）へ join、weight sweep。純 α=1.0 と本番ブレンド α=0.2 の両方で baseline(wc=0) 比 Δ を測定。

| | 純モデル α=1.0（最良 wc） | 本番ブレンド α=0.2（全 wc） |
|---|---|---|
| baseline | top1 0.1417 / AUC 0.6838 | top1 0.3153 / AUC 0.8356 |
| Δtop1 | **+0.0084**（wc=0.25） | **−0.0017**（flat・むしろ劣化） |
| ΔAUC | **+0.016**（wc=0.5） | **+0.0002**（≈0） |
| ΔBrier / ΔLogLoss | 全 wc 改善（−0.00014 / −0.00131 まで） | ≈0（−0.00001 / −0.00012） |

- **純モデルでは調教評価に本物の resolution 情報**（AUC +0.016・top1 +0.0084・Brier/LogLoss 改善）。血統（AUC +0.0011・top1 +0.0020）・脚質（top1 劣化）より明確に良い、棄却レバー中で最良の純シグナル。
- **しかし本番ブレンド(α=0.2)で完全吸収**（AUC +0.0002・top1 −0.0017）。「純では効くのにブレンドで消える」を直接可視化＝ADR 0027（市場ブレンドが精度の主レバー・公開ファンダは市場が織り込む）の教科書的実証。

## 理由

- **市場が調教を織り込む**（ADR 0027）。α=0.2 は市場 win に 0.8 の重みを与え、baseline blended AUC は 0.836（市場支配）。調教評価は市場自身の調教織り込みを超える増分を持たず、純の +0.016 AUC は blend で 5x 希釈されて消える。血統 0058・脚質 0061 と同じ死因だが、本 arc は「純 improves → blended flat」を最も clean に示す。
- **A〜D が吸収される ⇒ 生タイムも吸収の公算大**（a fortiori）。netkeiba の粗い主観グレードですら市場に織り込まれている以上、より詳細な JV-VAN 生タイム（6F/5F… 脚色/併せ馬）も市場が同等以上に織り込む。高コストな Windows/JV-Link 取得経路の確立・premium 課金に踏み込む価値はない。調教方向（公開データ）は closed。

## 留保

- cheap screen は 2ヶ月 593R（1 窓）＝top1 の SE ≈ 0.014 で純 top1 +0.0084 は 1SE 内。ただし**棄却の主根拠は純 top1 の有意性でなく blended での吸収**（AUC +0.0002・top1 −0.0017）で、これは市場 0.8 重みの構造から来るためサンプル拡大で反転しない。全量/多窓での再測定はコスト対効果で見送り。
- 短評（テキスト）の NLP 信号化は未測定だが、A〜D グレード（短評を編集部が要約したもの）が blend で吸収される以上、同根で吸収される公算が高い。

## 影響

- **本番コード・スキーマ・CLAUDE.md いずれも変更なし**。ADR と測定記録のみ（血統 0058 と同じ scratch-first 撤退）。
- 測定スクリプト（`/tmp/pa/fetch_cyokyo.py`・`cyokyo_prototype.py`）は本番外の使い捨て scratch でリポに残さない。再提案防止の記録として本 ADR に集約。
- 関連: 0027（精度の主レバー＝市場ブレンド・公開ファンダは市場が織り込む）/0058（血統棄却・factor 冗長性）/0059（市場較正棄却）/0061（脚質棄却）/0055（EV 層分離・執行エッジへ）。**純モデル resolution arc は「現行＋公開新データ天井」で確定的に closed。**

## 再現

```sh
# 1. 純 dump（18ヶ月・production 相当）: docs/adr/0061 と同じ
./target/release/paddock-analyze backtest --from 2025-01-01 --to 2026-06-30 \
  --blend-alpha 1.0 --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 \
  --dump-features /tmp/pa/pure_long.tsv
# 2. 調教評価 scrape（netkeiba oikiri・canonical race_id リスト・pacing 3s）
python3 /tmp/pa/fetch_cyokyo.py /tmp/pa/cyokyo_races.txt /tmp/pa/cyokyo_full.tsv
# 3. marginal-lift（純 α=1.0 と blend α=0.2 の sweep・忠実性 5.55e-17）
python3 /tmp/pa/cyokyo_prototype.py /tmp/pa/pure_long.tsv /tmp/pa/cyokyo_full.tsv
```
