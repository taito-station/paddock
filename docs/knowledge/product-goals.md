---
status: Confirmed
kind: knowledge
doc_class: [D01]
tags: [D01]
updated: "2026-08-16"
---

# プロダクト目標・成功条件・非目標（D01）

paddock が何を目指し、何を達成したら成功で、**何をやらないと決めたか**を 1 枚にまとめる
（ADR 0073 決定 4）。この文書ができるまで、プロダクトの方向性は ADR 73 本を読み解くことでしか
復元できなかった。

> **非目標が本文の過半を占めるのは意図的**。paddock の資産のうち最も再利用価値が高いのは
> 「測って採らなかった」記録（棄却 ADR 24 本）で、これを畳んでおかないと同じ検証を何度も
> 繰り返す。棄却の詳細は各 ADR が正、本文はその索引と分類。

## 目標

1. **数値で競馬を見る**。印象・話題ではなく、確率・期待値・実測で意思決定する。
   予想の各段は再現可能（同じ入力から同じ出力）で、根拠は数字に還元できる。
2. **買い方を楽しく、売れる形で提示する**。「当てる」ことではなく「そのまま買える買い目」と
   その根拠を提示することを成果物とする。読み手が買える形（式別 / 方式 / 軸 / 相手 / 点数 / 金額）で
   出し切ることを最低条件にする。

この 2 つは並列ではない。**1 が 2 の前提**で、数値の裏付けが無い買い目は提示物として成立しない。

## 成功条件（REQ）

各要件は `REQ-D01-NNN` で参照する（規約は [README.md](README.md) の「REQ-ID（要件 ID）の規約」節）。
**`status: Confirmed` の要件は検証手段が埋まっていることを機械検査で強制する**——「達成した」と
言えるのは測り方が決まっているときだけ。

<!-- REQ:begin D01 -->
| REQ-ID | 要件 | 検証手段 | 出典 | status |
|---|---|---|---|---|
| REQ-D01-001 | 張るレースは ROI ≥ 100% のものだけに限る。閾値は引き下げない。**ただし現行の参考ROIはこのゲートの判定指標として機能していない**（下記「ゲートの現況」）——ゲートを緩める根拠にはならないので、閾値の引き下げは引き続き行わない | ADR 0040 の再現方法（保存済み `race_odds` に `analyze predict --blend-alpha 0.2` と `scripts/predict-check/live_ev.py` を当てて全 3 券種の ROI 分布を出す。**`analyze predict` は集計統計に `as_of=None` を使うので過去レース再評価ではリークする**——ただし +EV を多く見せる向きなので「+EV 帯が薄い」という結論には保守的）を再実行し、**閾値を下げると −EV を買うことになる**ことを確認する。あわせて `scripts/predict-check/gate_calibration.py`（ADR 0076 の再現方法）で判定ROIと実現ROIの較正を測り直す。ADR 0040 時点の実測は 69R で平均 ROI 73.1% / 最高 97% / +EV 0 本、ADR 0076 の実測は 182R で判定ROI 平均 23.2% / 最高 76.8% / ゲート通過 0 本（**この測定は #621 の番兵除去前**。判定ROI の出所は `live_ev_snapshots.roi` ＝ predict-watch が保存済みの計算結果で、netkeiba の未発売番兵が EV を 3 桁にする経路はここ。**#621 の修正では直らない**ので、取り直しには番兵除去後の記録が要る（#625）。なお市場整合ROI 側は式の上で `o` が約分されるためほぼ無影響） | ADR 0040 / ADR 0076 / ADR 0079（表示と運用記述） | Confirmed |
| REQ-D01-002 | 順位付けは blended 確率、EV は純モデル確率 × 市場オッズで計算する（確率と買い方の層分離） | `cargo test -p paddock-domain` の EV 層テスト（純モデル確率が EV 経路に渡ることを固定） | ADR 0055 | Confirmed |
| REQ-D01-003 | 軸は事前データで確定し、直前オッズでは動かさない（用途はズレ増額のみ・軸フリップ禁止）。**軸だけでなく相手・混戦判定も固定する**（実装は REQ-D23-007） | `scripts/predict-check/gate_calibration.py` の「軸（◎）の安定性」節が **`0/N`**（発走前に 2 スイープ以上あったレースで軸が入れ替わらない）。目視のログ突き合わせでは取りこぼす——ADR 0078 以前は 154R 中 軸 28R・相手 62R が黙って入れ替わっていた。機械検査は `cargo test -p paddock-domain` の `pinned_selection_survives_market_movement_while_roi_moves`（選定は不変・ROI は動く）が張る | ADR 0060 / ADR 0078 | Confirmed |
| REQ-D01-004 | ADR 0052 と同一条件（α=0.2・縮約 / 冪較正フラグなし）のトップ選好馬の単勝的中率が 28% を下回らない（890R 実測 29.9%） | `paddock-analyze backtest --from 2026-03-15 --to 2026-06-21 --blend-alpha 0.2` の `win_hit_rate`（ADR 0052 の再現方法と同じコマンド。`backtest` は m / 冪較正を既定適用しないので、本番構成で測るならフラグを明示したうえで閾値ごと測り直す） | ADR 0052 | Confirmed |
| REQ-D01-005 | 同条件のトップ選好馬の複勝的中率が 60% を下回らない（890R 実測 64.5%） | 同上（`show_hit_rate`） | ADR 0052 | Confirmed |
| REQ-D01-006 | 手動ハンデ軸精査を伴う実運用セッションの単勝的中率が、REQ-D01-004 と同条件のバックテスト水準（29.9%）を上回る＝エッジが実在することを実測で示す | 実運用セッションの `bet_records` と結果照合を 200R 以上貯め、`◎` の単勝的中率をバックテストと同じ定義で集計する（現状の観測は 1 開催日規模で母数が足りず、確定知にできる水準にない） | ADR 0055（エッジ＝手動ハンデ軸精査という主張の出所） | Tentative |
| REQ-D01-007 | 買い目は「そのまま買える形」で提示する（式別 / 方式 / 軸 / 相手 / 点数 / 金額・100 円単位） | `build_portfolio` の単体テスト（配分の正）と [live-ev-buy-view.md](../specifications/live-ev-buy-view.md) の**表示形式（slip）**の契約（配分方式の正は `build_portfolio`。ライブ writer は #346 で Rust `predict-watch` に一本化済み） | ADR 0064 | Confirmed |
| REQ-D01-008 | 予想と買い目はブラウザから閲覧できる（ローカル完結・外部ストレージに依存しない） | `docs/api/openapi.json` のスナップショット検証（`src/apps/api-server/tests/openapi.rs`）と `web/src/lib/board.test.ts`、および [race-list-dashboard.md](../../tests/browser-test-cases/race-list-dashboard.md) の手動ブラウザ手順 | ADR 0069 | Confirmed |
<!-- REQ:end D01 -->

## エッジの所在

**「市場より上手く勝者を当てる」路線は閉じている**（ADR 0027 → 0052 → 0058/0059 で確定）。
純モデルの単勝的中は 12.0%、市場は 29.7%、本番の α=0.2 ブレンドは 29.9% で、**現行精度は事実上
ほぼ市場由来**。20% のモデル重みは本命選択をほとんど動かしていない。

したがって paddock のエッジは 2 か所にしかない。

1. **手動ハンデ軸精査**——コース/枠バイアス・近走フォーム・距離/騎手という、馬柱に出るが
   市場が完全には織り込まない材料を人間が読む部分（ADR 0055 理由節）。
2. **執行の規律**——軸ロック＋ズレ増額（ADR 0060）。pari-mutuel は必ず最終プールオッズで買うので
   価格エッジは存在せず、直前情報の正しい使い道は「同じ軸をより美味しく買う」ことだけ。

**この 2 つ以外に勝ち筋を探しに行かない**というのが、以下の非目標の実質的な意味になる。

### ゲートの現況（ADR 0076・182R 実測）

EV 層分離（ADR 0055）後の参考ROIは、**レースを選別するゲート指標として機能していない**。
2026-07-11〜08-09 の 182 レース（`live_ev_snapshots` の記録済み判定と買い目伝票を確定払戻で精算）で:

- **ゲート通過 0 件**。判定ROIの最大は 76.8%、平均 23.2%。100% は構造的に到達不能。
- **判定ROI ÷ 市場整合ROI(=1−控除率 77.0%) = 0.30**。買い目の脚は blended（市場優位）で選ぶのに
  EV は市場情報を捨てた pure 確率で値付けするため、選ばれた脚が構造的に低く出る＝**閾値でなく定義の不整合**。
- **Spearman(判定ROI, 実現ROI) = +0.002**。ADR 0044 の分離前定義（−0.167＝逆予測的）と違い、
  逆predictではなく**無情報**。EV 層分離は病理を取り除いたが選別力は与えていない。
- 無ゲート実現ROI 69.5%（95% CI 56.6〜83.2%）は市場整合 77.0% と区別がつかない＝この窓にエッジは無い。

したがって **「ROI ≥ 100% のレースだけ張る」を自動判定として当てにしない**。判定は上記 2 つのエッジ
（手動ハンデ軸精査・執行の規律）で行い、参考ROIは decision-support の材料に留める（ADR 0055 決定 4 / ADR 0060）。

## 非目標

24 本の棄却 ADR（`*-rejected.md`）から復元した「測ったうえで採らないと決めたこと」。
**再提案するときは、その ADR の検証条件を上回る新しい測定を先に用意する**（同じ土俵の再議論はしない）。

### A. 純モデルの予測精度を上げる（resolution 天井・factor 冗長性）

| ADR | 採らなかったこと | 棄却の理由（要約） |
|---|---|---|
| 0034 | recency（時間減衰）重み | Brier/LogLoss が変わらず ROI も誤差範囲。複雑性だけ増える |
| 0035 | `recent_form_weight` の再チューニング | 4891R で 0.25 を**有意に**上回る値が無い（最良でも LogLoss 差 0.0001〜0.0003 で過剰適合リスクに見合わない） |
| 0036 | 直近 N 走のトレンド加重平均 | 893R で N=1（前走のみ）を上回らない |
| 0038 | 騎手の直近フォーム factor | 重み 0 が最良。機構だけ残す |
| 0050 | place/show 素スコアの m×recency×form joint retune | 脱圧縮は ADR 0047 の冪変換で足り、joint retune に上積みが無い |
| 0052 | 市場ブレンドの廃止（純モデル化） | 純モデルは単勝 12.0% で市場 29.7% の半分以下 |
| 0053 | 学習型モデル（条件付きロジット/PL・GBM）への置換 | モデルクラスを変えても市場を超えない |
| 0058 | 血統（種牡馬）適性 factor | 現行データの天井内でノイズ級 lift。**天井の主因＝factor 冗長性を確定させた ADR** |
| 0061 | 脚質（先行度）factor | AUC/校正は微改善するが本命指標 top1 が全 weight で劣化 |
| 0062 | 調教（追い切り）評価 factor | 純モデルでは情報を持つが市場ブレンドに完全吸収される |

### B. 市場のミスプライスを突く

| ADR | 採らなかったこと | 棄却の理由（要約） |
|---|---|---|
| 0037 | place/show・exotic の市場ブレンド本番化 | 校正は微改善するが回収率が悪化。exotic は本番経路に届かない |
| 0059 | 人気-穴バイアスの較正補正 | バイアスは実在するが takeout（19%）より小さく exploitable でない |
| 0063 | エキゾ（馬連/3連複/馬単）のミスプライス収穫 | 額面 +EV は 83R・実質 5 開催日の小標本変動 |
| 0067 | late money / 単勝 log-odds drift のシグナル化 | クロージングラインが動きの情報を既に吸収済み |

### C. 買い方（券種・配分・ゲート）を広げる・複雑にする

| ADR | 採らなかったこと | 棄却の理由（要約） |
|---|---|---|
| 0028 | 混戦判定へのオッズ条件併用 | baseline を上回る閾値が 1 つも無い |
| 0030 | 混戦時の 3 連複の相手拡大（top7・全頭） | 的中率は上がるが回収率は全 variant で低下 |
| 0033 | 条件付き単勝の追加 | 全閾値・全予算モードで 3 券種 baseline を下回る |
| 0039 | 上位近接時の 2 軸フォーメーション | ◎1 頭軸ながしを上回らない |
| 0040 | EV ゲート閾値（ROI ≥ 100%）の引き下げ | ROI 100% が損益分岐そのもの。緩めると −EV を買う |
| 0041 | 馬連特化戦略 | 3 券種ポートフォリオを上回らない |
| 0043 | 馬単のポートフォリオ導入 | 導入後 71R の検証で純損と確定し #271 で撤回 |
| 0046 | 配分の確率重み化＋脚ごと最低 ¥100 の撤廃 | 薄い脚への少額 spread が実 ROI を上げている |
| 0054 | fractional Kelly 配分 | 同一土俵の比較で現行ヒューリスティックを上回らない |

### D. 技術スタックの入れ替え

| ADR | 採らなかったこと | 棄却の理由（要約） |
|---|---|---|
| 0071 | `web/` React SPA の Topcoat（SSR フルスタック）置き換え | 型境界の消滅という利点はあるが、今取るには移行コストが見合わない（reject-for-now） |

### E. 文書として書かないこと

- **収益化の具体（価格・販路・課金導線）は docs に書かない**。目標 2 の「売れる形で提示する」は
  提示物の品質要件までを指し、値付けと販路はリポジトリの管理対象外とする（ADR 0073 決定 4）。
- **second source を作らない**（ADR 0064）。買い方ロジックの正は Rust `build_portfolio` で、
  `scripts/predict-check/` の Python 実装はオフライン EV レポート専用。

## この文書の更新条件

- **目標が変わったとき**、および**非目標が覆ったとき**（＝棄却 ADR を supersede する新しい ADR が
  承認されたとき）に更新する。棄却の再提案そのものは ADR 側で行い、ここには結果だけを写す。
- 成功条件の数値は、母数を更新したバックテストが出たら `検証手段` の再実行結果で置き換える
  （窓を変えたら閾値も併せて測り直す）。
- **本文は棄却 ADR の索引であって写しではない**。`sources` が 31 本あるのは網羅の宣言で、
  追従が要るのは**出典の決定が変わったとき**（supersede）だけ。ADR は不変なので、機械置換のような
  内容を変えない変更で stale が出た場合は [README.md](README.md) の例外 1/1b/1c に従う。
- 関連: [doc-classes.md](doc-classes.md)（クラス体系）/ [README.md](README.md)（蒸留規約・REQ-ID 規約）/
  [betting-rule-history.md](../specifications/betting-rule-history.md)（買い方ルールの決定根拠と棄却履歴）

---

## 決定ログ

<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->

### ADR 0062: 調教（追い切り）評価 factor は市場ブレンドに吸収され不採用（棄却） (2026-07-02) — 棄却

#### ステータス

棄却（#327 調教データの marginal-lift 測定 arc・measure-first）。本番コード変更なし（配管ゼロで撤退）。血統 0058・市場較正 0059・脚質 0061 に続き、**「現行＋取得可能な公開新データ（血統/脚質/調教）は全て市場ブレンド(α=0.2)に吸収される」**を確定＝純モデル resolution 路線は完全に closed（ADR 0027）。次の伸び代は執行エッジ（ADR 0055/0060）のみ。

#### コンテキスト

純モデル resolution の残る唯一の未探索 fundamental レバーが調教（追い切り）（#327）。issue は JV-Link（Windows COM SDK）の生タイムを想定していたが、取得経路の確立が非自明（この PJ は macOS/Lima 中心）。

調査の結果、**netkeiba 追い切りページ `race.netkeiba.com/race/oikiri.html?race_id=<12桁>` が無料で「調教評価(A〜D)＋短評」を HTML テーブルで提供**（全出走馬・archived で as-of 安全・1レース1fetch で激安）と判明。JV-Link を回避し、既存 netkeiba scraper と同じ経路で cheap screen できる。粗い主観グレードだが「これが効かないなら生タイム（premium/JV-VAN）も効かぬ公算が高く安く切れる」との判断で、**最小コストの netkeiba 無料 A〜D 評価から measure-first**（血統 arc ADR 0058 と同型・scratch-first）。物差しは AUC/top1/Brier（ROI でない・ADR 0055）。

#### 決定

調教評価 factor を**本番採用しない**。純モデルでは明確な resolution 情報を持つ（血統/脚質より良い）が、**本番の市場ブレンド(α=0.2)で完全に吸収される**（市場が調教評価を既にオッズへ織り込む・ADR 0027）。粗い A〜D すら吸収される以上、高コストな JV-VAN 生タイムも吸収される公算が極めて高く、調教方向（公開データ）全体を見送る。

#### 検証（measure-first・cheap screen で撤退）

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

#### 理由

- **市場が調教を織り込む**（ADR 0027）。α=0.2 は市場 win に 0.8 の重みを与え、baseline blended AUC は 0.836（市場支配）。調教評価は市場自身の調教織り込みを超える増分を持たず、純の +0.016 AUC は blend で 5x 希釈されて消える。血統 0058・脚質 0061 と同じ死因だが、本 arc は「純 improves → blended flat」を最も clean に示す。
- **A〜D が吸収される ⇒ 生タイムも吸収の公算大**（a fortiori）。netkeiba の粗い主観グレードですら市場に織り込まれている以上、より詳細な JV-VAN 生タイム（6F/5F… 脚色/併せ馬）も市場が同等以上に織り込む。高コストな Windows/JV-Link 取得経路の確立・premium 課金に踏み込む価値はない。調教方向（公開データ）は closed。

#### 留保

- cheap screen は 2ヶ月 593R（1 窓）＝top1 の SE ≈ 0.014 で純 top1 +0.0084 は 1SE 内。ただし**棄却の主根拠は純 top1 の有意性でなく blended での吸収**（AUC +0.0002・top1 −0.0017）で、これは市場 0.8 重みの構造から来るためサンプル拡大で反転しない。全量/多窓での再測定はコスト対効果で見送り。
- 短評（テキスト）の NLP 信号化は未測定だが、A〜D グレード（短評を編集部が要約したもの）が blend で吸収される以上、同根で吸収される公算が高い。

#### 影響

- **本番コード・スキーマ・CLAUDE.md いずれも変更なし**。ADR と測定記録のみ（血統 0058 と同じ scratch-first 撤退）。
- 測定スクリプト（`/tmp/pa/fetch_cyokyo.py`・`cyokyo_prototype.py`）は本番外の使い捨て scratch でリポに残さない。再提案防止の記録として本 ADR に集約。
- 関連: 0027（精度の主レバー＝市場ブレンド・公開ファンダは市場が織り込む）/0058（血統棄却・factor 冗長性）/0059（市場較正棄却）/0061（脚質棄却）/0055（EV 層分離・執行エッジへ）。**純モデル resolution arc は「現行＋公開新データ天井」で確定的に closed。**

#### 再現

```sh
# 1. 純 dump（18ヶ月・production 相当）: docs/original-docs/0061 と同じ
./target/release/paddock-analyze backtest --from 2025-01-01 --to 2026-06-30 \
  --blend-alpha 1.0 --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 \
  --dump-features /tmp/pa/pure_long.tsv
# 2. 調教評価 scrape（netkeiba oikiri・canonical race_id リスト・pacing 3s）
python3 /tmp/pa/fetch_cyokyo.py /tmp/pa/cyokyo_races.txt /tmp/pa/cyokyo_full.tsv
# 3. marginal-lift（純 α=1.0 と blend α=0.2 の sweep・忠実性 5.55e-17）
python3 /tmp/pa/cyokyo_prototype.py /tmp/pa/pure_long.tsv /tmp/pa/cyokyo_full.tsv
```

### ADR 0063: エキゾ（馬連/3連複/馬単）ミスプライス収穫の検証 — 現データでは棄却（兆候は小標本変動） (2026-07-03) — 棄却

#### ステータス

棄却（reject-for-now）。本番 `live_ev` 経路への接続はしない。手法・スクリプトは残し、リーク無しの
過去エキゾオッズ＋結果が数百 R 規模に蓄積した時点で再検証する（下記「再検証の条件」）。

#### コンテキスト

ADR 0052/0053（#309）で「純モデル・学習型 fundamental モデルは**単勝市場に勝てない**（市場が過去走
fundamental を包含）」と walk-forward で確定し、単勝の win 確率を当てに行く筋は閉じた。残るエッジ候補は
「群衆が組合せ確率を正しく合成できず**ミスプライスが残りやすい派生市場**（馬連/ワイド/3連複）」＝
Benter/Ziemba のエッジ源泉。

そこで #314 では**新規モデルを作らず**、既に信頼できる **市場ブレンド単勝確率（production α=0.2・as-of）
を Plackett-Luce/Harville でエキゾ組合せ確率に展開 → 実エキゾオッズと突合し、控除率を net で抜けて +EV に
なる券種・帯があるか**をリーク無しで検証した。勝てなければ素直に棄却する（#309 と同じ規律）。

#### 検証方法

- **リーク無し win%**: `analyze backtest --blend-alpha 0.2 --shrinkage-m 10 --win-power 1.25
  --place-show-power 2.0 --dump-features` の `model_win`（as-of＝`races.date < D` を強制。`analyze predict`
  は all-time 統計でリークするため不使用）。
- **合成**: `umaren_backtest.py` の `p_top2_set`（馬連）/`p_top3_set`（3連複）/`p_exacta`（馬単）を再利用。
- **EV とミスプライス**: `EV = P_synth × DB オッズ − 1`。選抜は `EV ≥ θ`。**清算は netkeiba 実配当**
  （DB オッズは EV 選抜のみ＝循環回避 #250）。
- **集計**: 券種別／予測EV帯別（較正）／頭数帯別／組合せオッズ帯別／**開催日別（安定性）**、および
  baseline「全組合せ均等買い」。実装は `scripts/predict-check/exotic_mispricing.py`。

##### 母集団（feasibility の強い制約）

リーク無しに使えるのは **83R / 5 開催日（2026-03-15, 05-30, 05-31, 06-13, 06-14）** のみ。理由: win% を
as-of で得る `analyze backtest` は results テーブルのある race（〜2026-06-14）しか出せず、その ∩ エキゾ
オッズ有り = 83R。**ワイドは結果と結線できるのが 12R のみ**（#248 既知）で検証不能＝本 ADR ではスコープ外。
JRA 控除率は 3 連系 ~27.5% / 馬連 ~22.5% で、net で抜くバーは単勝(~20%)より高い。

#### 結果（83R）

baseline（全組合せ均等）は **馬連 75.2%・馬単 68.5% が `1 − 控除率`（~77.5% / ~75%）にほぼ一致**し
パイプラインの健全性を確認できる。3連複は 43.7% と期待（~72.5%）より低いが、これは母集団欠落では
**ない**（各券種とも**当たり組合せは 83R 全てで DB オッズに存在**＝ 100%。均等買いは必ず的中を含む）。
3連複は組合せ空間が C(n,3) と広く、83R では実現払戻の分散で `1 − 控除率` から下振れているだけで、
本検証の母集団バイアスを示すものではない:

| 券種 | baseline(均等) | 合成EV≥0 | EV≥0.5 | 開催日別 ROI（EV≥0.5） |
|---|---|---|---|---|
| 馬連 | 75.2% | 138.5% | 242.1% | 0% / **2560%** / 0% / 318% / 131% |
| 3連複 | 43.7% | 57.0% | — | 4% / **1232%** / 6% / 65% / 94% |
| 馬単 | 68.5% | 74.0% | 56.1% | 0% / 0% / 0% / **144%** / 0% |

- **馬連は額面 +EV（138〜242%）が出る**が、頑健でない:
  - **開催日で崩壊**: 5 日中 2 日が実現 0%、1 日（05-30, n=26）が 2560% で全体を牽引。leave-one-out で
    05-30 を除くと EV≥0.5 は 242%→172%、上位 2 日を除くと **67%** に落ちる（＝2 日の幸運に依存）。
  - **的中率 0.2〜0.5%** の超低頻度＝分散が支配的。選抜されるのはほぼ大穴（オッズ≥100 帯が n の大半）で、
    「大穴組合せが PL 合成比で過小人気」を数点の的中が偶然実現しただけ。
  - **予測EV帯→実現ROI が単調でない**（<-0.5:87% / -0.5〜-0.2:38% / -0.2〜0:36% / 0〜0.5:54% /
    0.5〜1:274% / ≥1:220%）。信号が本物なら EV 帯とともに単調に上がるはずだが、そうならない。
    per-bet Spearman は 0 付近（返り値の 99% が 0＝タイ支配で情報量に乏しい）。
- **3連複・馬単は集計で 100% を抜けない**（57% / 74%）。より高い控除率のバーを越えず、やはり単一日
  （3連複 05-30 / 馬単 06-13）だけが正で他日はほぼ 0%。

#### 決定

**現データでは棄却**。額面 +EV は **83R・実質 5 開催日の小標本 × 大穴超低頻度**が生む変動で、控除率を安定して
抜くエッジは実証できない。特に「単一〜2 開催日が全体を牽引・他日は 0%・予測EV帯が単調でない」は、
ADR 0044/0045 の model-EV ゲート逆予測や ADR 0033/0041 と同型の「較正不良ゾーンでノイズを掴む」パターン。
本番 `live_ev` 経路には接続しない。

#### 再検証の条件（reject-for-now）

馬連の額面 +EV は「大穴組合せの過小人気」という古典的に有り得る機序で、頭ごなしに否定はしない。ただし
控除率 22.5% を安定して抜くには **リーク無しの過去エキゾオッズ＋結果が最低でも数百 R（できれば正の複数
開催を含む窓）** 必要。現状はエキゾオッズ蓄積が 2026-03 以降・結果結線が 5 日分しかなく不足。#248 の
snapshot 蓄積（あるいはエキゾ結果の遡及取得）が数百 R 規模に達したら、本 ADR のスクリプトで再検証する。

#### 影響

- `scripts/predict-check/exotic_mispricing.py`（新規）: as-of dump→PL 合成→実オッズ突合→券種/帯/日別
  net-ROI 集計。`umaren_backtest.py` の合成確率・パース・配当を import 再利用。
- `scripts/predict-check/test_exotic_mispricing.py`（新規・standalone）: dump パース・券種振り分け・
  EV/実配当清算・quinella↔umaren 払戻マッピング・ROI・オッズ帯の不変量。
- `scripts/predict-check/gen_win_backtest_data.sh`: エキゾオッズ TSV 生成ステップ（`bet_type IN
  quinella/trio/exacta`）を追加。
- 本番定数・CLAUDE.md・買い方ルールは不変。

#### 関連

- 出自: #314（#272 配下「派生市場でのエッジ探索」）。
- 規律: #309/ADR 0052/0053（学習・純モデルは単勝で市場に劣る＝素直に棄却）。
- 同型の逆予測/小標本ノイズ: ADR 0044/0045（model-EV ゲート逆予測）, ADR 0033/0041。
- データ: #248（発走直前オッズの年間スナップショット蓄積）, #218（race_odds_snapshots）。
- 補足候補（別 issue）: #315（late money シグナル）, ADR 0054（Kelly 配分は棄却済）。

### ADR 0067: オッズの動き（late money / 単勝 log-odds drift）のシグナル化 — 現データでは棄却（クロージングが吸収） (2026-07-10) — 棄却

#### ステータス

棄却（reject-for-now）。予測・EV 選抜への組み込みはしない。手法・スクリプト（`scripts/predict-check/late_money_probe.py`）は残し、
リーク無しの複数時点オッズ＋結果が数百 R 規模に蓄積した時点で同スクリプトで再検証する（下記「再検証の条件」）。

#### コンテキスト

「朝の +EV は発走直前に剥がれる」（市場が締まると妙味が消える）は運用知見として確立している（project memory）。
これを裏返し、**オッズの締まり方そのものが情報**になりうるか＝発走直前に支持を集める（オッズが下がる）馬は、
調教・厩舎の自信・資金流入など**市場が見て過去走 fundamental が見ない情報**（ADR 0027）を反映した「賢い金（late money）」
ではないか、を #315 で検証した。

ADR 0052/0053（#309）で「静的な過去走 fundamental は単勝市場に勝てない」と確定した後の、**市場自身の時間変化**という
これまでの軸と根本的に異なる情報源のエッジ探索（#272 配下）。#315 が最初に課したのは
「`race_odds_snapshots` に朝〜直前の複数時点オッズがどれだけ蓄積されているか」の feasibility 判断。

#### feasibility 棚卸し（第一関門）

`race_odds_snapshots`（#218 経路で `odds-collect`/`predict-watch` が蓄積）を実測:

- **signal 側は十分**: 単勝で **≥2 時点 かつ 朝→直前スパン ≥30 分のレースが 94R**（総 135R 中）。特に 2026-07-03 開催は
  全レースが 12〜24 時点のリッチ解像度。log-odds drift・支持順位変化の特徴量化は成立する。
- **label 側は DB 未整備**: これらの snapshot 対象日（函館/福島/小倉・06-27〜07-05）は snapshot だけ溜め、通常の予想
  ワークフロー（fetch-card + 結果取り込み）を回していないため `races`/`results` に未取り込み。DB の `results` は 06-14 まで。
  race_id スキームは全テーブル共通スラッグで**不一致ではない**（単に未取り込み）。
- 対処: 過去レースなので着順は `scripts/predict-check/nk.py` の実証済みパーサ `fetch_result`（枠番/馬番の誤検出対策・空取得の警告つき）で netkeiba 結果から取得し、DB の snapshot オッズと突合。**94R 全て結果取得成功・欠落 0**。

#### 検証方法

`scripts/predict-check/late_money_probe.py`（新規・standalone・標準ライブラリ + 同ディレクトリ `nk.py` の実証済み netkeiba ヘルパを再利用）:

- **シグナル**: 各馬の単勝 `drift = ln(odds_first) − ln(odds_last)`（>0＝締まった＝金が入った）。first=最古 snapshot、last=最新 snapshot。
- **baseline（クロージングライン）**: 最新 snapshot の単勝オッズをレース内で控除除去正規化した `p_last`（市場の到達勝率）。
- **リーク無し**: drift も終値も**発走前に確定**する情報で bet 時点で利用可能（過去走と違いリークでない）。評価は当時 snapshot で厳密に。
- **問いの設計**: 「drift が着順を予測するか」ではなく「**終値 `p_last` が既に織り込んだ水準を超えて** drift が着順/勝敗の残差を説明するか」。
  これがゼロなら「動いた後には妙味なし」で late money は執行エッジにならない。

#### 結果（94R / 1,187 頭）

| テスト | 結果 | 解釈 |
|---|---|---|
| ① 生の相関 Spearman(drift, 着順) | **−0.378**（勝敗 +0.215） | 締まる馬ほど好走に見える |
| （参考）Spearman(終値人気, 着順) | −0.548 | だが①は交絡: 締まる馬＝人気馬（drift 五分位で終値 implied が 0.016→0.164 と単調上昇）。かつ着順はレース跨ぎでプールしており出走頭数差も混ざる |
| ② Spearman(drift, 残差 実勝敗−終値 implied) | −0.509 | >0 なら終値超えの予測力。負だが**この残差も `p_last` と drift の共線で level を完全には抜けない**（③の五分位内比較の方が交絡制御として妥当）ため単独では決め手にしない |
| ③ drift 五分位・残差（実勝率 − 終値 implied） | +0.005 / −0.012 / −0.019 / **+0.046** / −0.017 | **小さく非単調**。締まった馬が終値を超えて走る/凡走する単調エッジは無い |
| ④ **logloss: 終値のみ vs drift 加味（1 変数ロジ）** | **最適 drift 係数 b=0.00・Δlogloss=0.0000** | **終値に drift を足しても情報ゼロ** |

- **決め手は③と④の合わせ技**（①②は level 交絡が残るので補助）。③は終値水準を五分位で揃えても残差が非単調（方向性なし）、
  ④は `logit(p_last) + b·drift` を b について in-sample グリッド探索（[−3,+3]・0.1 刻み）しても logloss を下げる b が
  **厳密に 0**。過学習の自由度を与えてすら drift は終値を改善しない。なお④は各レースで Σp=1 の再正規化を省いた近似で、
  改善の**検出力はやや保守的**（null 側に寄る）——ただし b=0 のとき alt は厳密に `p_last` に一致するので「最適 b=0」の結論は
  正規化の有無に依らず成立する。
- ① の見かけの相関は純粋に「steamer＝人気馬」の **level proxy**。level（終値人気）を制御すると（③④）増分は消える。
- ③ の Q4（drift≈0 の帯）だけ残差 +0.046 が目立つが、最も締まった Q5 は −0.017 で単調でなく、方向性のある信号ではない。

#### 決定

**現データでは棄却**。**クロージングライン（最終 snapshot の市場勝率）がオッズの動きの情報を既に吸収しており、
drift に終値を超える増分予測力は無い**（level 制御下の五分位残差が非単調＝③、かつロジで drift の最適重み＝0＝④）。市場効率の教科書的帰結であり、
既存の天井知見（project memory「朝の +EV は直前で剥がれる」＝クロージングが最良価格 / ADR 0053 の単勝市場優位 /
ADR 0058-0063 の公開情報系 factor 棄却系譜）と整合する。予測・EV 選抜・買い方ルールには一切接続しない。

執行エッジは引き続き **軸ロック＋ズレ増額**（ADR 0055/0060）に置く。「オッズが動いた方向を追う」のではなく
「自モデル軸に対して終値が過小人気にズレたら増額」——本 ADR は前者に妙味が無いことを確認し、後者の規律を補強する。

#### 留保・再検証の条件（reject-for-now）

- **n=94R は薄め**。ただし「in-sample の過学習自由度込みで最適重み＝0」は負の頑健性が高い。
- **「最終 snapshot」は post time 未取得のため真の直前とは限らない**。ただし本検証は「**我々が実際に観測・執行できる
  最後の価格**に対して drift が情報を足すか」という運用上正しい問いに答えている（真の直前ならむしろ更に吸収する方向）。
- snapshot は `predict-watch` で今後も自然に蓄積。**リーク無しの複数時点オッズ＋結果が数百 R 規模**（正の複数開催を含む窓）に
  達したら、`late_money_probe.py` を再走して再評価する。それまでは棄却を維持。
- 別角度（例: 朝オッズしか使えない前提での「bet early」エッジ、steam の券種横断ミスプライス）は本 ADR のスコープ外。

#### 影響

- `scripts/predict-check/late_money_probe.py`（新規）: DB snapshot→first/last drift 算出→着順取得（`nk.py:fetch_result` を
  再利用・着順を JSON でローカルキャッシュ）→level 制御下の相関・五分位残差・1 変数ロジ logloss。再実行可能・依存は
  標準ライブラリ + 既存 `nk.py`。
- 本番定数・`CLAUDE.md`・買い方ルール・`live_ev` 経路は不変。

#### 関連

- 出自: #315（#272 配下「市場の動的情報でのエッジ探索」）。
- 規律: #309/ADR 0052/0053（純モデルは単勝で市場に劣る＝素直に棄却）。
- 天井系譜: ADR 0058/0061/0062/0063（公開情報系 factor は市場ブレンドに吸収）、project memory「朝の +EV は直前で剥がれる」。
- 執行エッジ: ADR 0055/0060（EV 層分離・軸ロック＋ズレ増額）。
- データ: #218/#248（`race_odds_snapshots` の複数時点蓄積）。

### ADR 0071: Topcoat（tokio-rs の SSR フルスタックフレームワーク）評価 — `web/` SPA の置き換えは棄却（reject-for-now） (2026-07-30) — 棄却

#### ステータス

棄却（reject-for-now）。`web/` SPA の Topcoat 置き換えは採らない。コード変更なし・本 ADR は評価記録のみ。
評価基準日は **2026-07-30**。下記「再評価の条件（reject-for-now）」が満たされたら再評価する。
**再評価の結果として決定が変わる場合に限り**、本 ADR を superseded する後続 ADR を起票する
（再評価しても棄却なら本 ADR を維持）。本 ADR 自体は不変記録として残す。

#### コンテキスト

2026-07-22 に tokio-rs から [Topcoat](https://github.com/tokio-rs/topcoat) がアナウンスされた。
「Rust でフルスタック reactive web app を書く」フレームワークであり、paddock の非 Rust 部分
（React SPA・Python スクリプト群・シェル）を Rust に寄せられるかを検討した。

##### Topcoat の事実確認（2026-07-30 時点・出典は「関連」節に commit ピンで記載）

- crates.io 最新 **0.5.0（2026-07-27 公開）**、MIT、初版 0.0.0 は 2026-04-17、累計 DL 2,466。
  0.0.1〜0.5.0 が 2026-07-14〜2026-07-27 の 2 週間に 13 版＝**動きが非常に速い**。
- **MSRV は crates.io メタデータで 1.95**（README には記載がない。edition 2024）。
  本 repo の `rust-toolchain.toml` は 1.97.1 なので**充足済み＝評価上の懸念ではない**。
- **完全サーバレンダリング**。async コンポーネントが DB を直接叩ける（別 API 層のボイラープレートを消す設計）。
- **WASM を使わない**。マクロで型検査済みの Rust 式を JS にクロスコンパイルし、HTMX 的な
  "reactive instructions" をメタデータとして配ることでクライアント反応性を足す。
  Leptos / Dioxus（WASM 系）とは狙う対話性の水準が違うと明言。限界時は HTMX / Alpine.js 統合にフォールバック。
- 同梱: `topcoat` CLI（dev server / `fmt` / `ui` / asset bundling）、content-hash ベースの asset pipeline、
  Tailwind ベースの shadcn/ui 風コンポーネント群、Fontsource / Iconify 統合、request-level memoization。
- **ランタイムに Node 不要**（完全サーバレンダリング＋WASM 非使用という上記 2 点の帰結）。
- **Tailwind 統合は default 外の opt-in feature**。crates.io 0.5.0 の `default` は
  `asset` / `compression` / `cookie` / `font` / `icon` / `router` / `runtime` / `serve` / `session` / `view` / `discover` で、
  `tailwind` は `dep:topcoat-tailwind` の optional（`tailwind.md`:「Enable the `tailwind` feature for both your
  runtime dependency and your build dependency」＋ `build.rs` の追加が必要）。
  **この feature を有効にした場合に限り**、ビルド時に既定で GitHub から standalone Tailwind CLI を
  `OUT_DIR` にダウンロードする（Tailwind 統合は "a thin Rust wrapper around the standalone Tailwind CSS CLI" で
  "It does not run Node, `PostCSS`, or a Vite-style asset pipeline"）。`BuildConfig::executable()` で
  preinstalled CLI を指定すれば "no download happens and no network access is needed"。
  外部 action を commit SHA でピンし cargo を `--locked` で固定する本 repo の CI 規律との擦り合わせが必要なのは、
  **`tailwind` feature を使う場合だけ**である。
- ルーティングは **既定は明示パス属性**（`#[page("/users/{id}")]`）。加えて `module_router!` マクロが
  "the recommended way to define routes" として提供され、こちらは **URL をモジュール木から導出**する
  （README の例: `src/app/posts/id.rs` → `/posts/{post_id}`）。**モジュール木＝URL は推奨形であって必須ではない**。
  エントリは `topcoat::start(Router::builder().discover().build()).await`。
- ルータは Topcoat 自前。optional な `tower` feature で tower service（axum router 等）を組み込める。
- README に **"Early-stage and experimental. Expect breaking changes."** と明記。
  アナウンス記事も「クライアント反応性は still in early stages」と自認。
- README のロードマップには **`OpenAPI` endpoints**、**(More) reactivity（`topcoat-runtime`）**、**Islands**、
  Streaming SSR / Suspense、Client-side navigation + prefetching、Toasty（tokio-rs の ORM）統合の強化、
  Static export、Authentication 等が並ぶ。**下記の見送り理由 2 と 3 は、このうち
  `topcoat-runtime` / Islands と `OpenAPI` endpoints が実装されれば弱まる**——再評価の観測点はここに置く。

##### paddock 側の非 Rust 部分の棚卸し（tracked files・実測基準 `main` = `409e4a4`）

| 領域 | 規模 | Topcoat の射程 |
|---|---|---|
| `web/` React 19 + TS + Vite SPA | `web/src` 39 files・8,904 LOC（.tsx 16 / .ts 22 / .css 1）。内訳は生成物 `web/src/api/schema.d.ts` 2,263 ／ 手書き CSS `web/src/styles.css` 990 ／ テスト `*.test.ts` 1,692 ／ **手書きアプリコード 3,959 LOC**。ほかに `web/` 直下の設定 7 files（`package.json` / `vite.config.ts` / `eslint.config.js` / `tsconfig.json` / `index.html` / `package-lock.json` / `.gitignore`） | ◎ 唯一の候補 |
| `scripts/predict-check/` | `.py` 37 files（オフライン EV レポート・backtest データ生成・各種 probe） | × Web でない |
| `tools/mdq/` | `.py` 17 files（BM25 ローカル索引・検索） | × 無関係 |
| `scripts/harness/` | `.py` 6 files（faithfulness チェック等） | × 無関係 |
| シェル（横断カテゴリ・上記ディレクトリと一部重なる） | `*.sh` 18 files（`deployments/` 3 / `scripts/` 直下 9 / `scripts/predict-check/` 5 / `scripts/harness/` 1）＋ 拡張子なしの bash 2 files（`scripts/mdq` / `scripts/git-hooks/pre-push`） | × 無関係 |

Python 行の件数は `.py` のみの数（tracked 総数は順に 43 / 20 / 10）。最終行はディレクトリ横断のカテゴリで、
上のディレクトリ行と一部重なる。**Topcoat の射程に入るのは `web/` だけ**で、検討対象は SPA 一点に絞られる。

本 ADR に載せた LOC / ファイル数は**すべて `main` = `409e4a4` 時点のスナップショット**である
（ADR は不変記録なので後続の変更で追随させない。再測するときはこの基準 commit と比較する）。

#### 決定

**`web/` の React SPA を Topcoat へ置き換えない。**（api-server / rest-controller も現状維持。）

#### 理由

##### 置き換えれば得られたはずの利点（評価済み・それでも今は取らない）

1. **型境界の消滅**。現在は `docs/api/openapi.json` → `openapi-typescript` → `web/src/api/schema.d.ts` →
   `openapi-fetch` という生成チェーンで型を渡している。Topcoat の DB 直読み構成なら DB〜画面まで単一の
   `cargo check` で通る（＝この利点は API 層ごと廃止する構成でのみ得られる。後述「却下した代替案」参照）。
2. **Node 依存木の廃棄**。react / react-router / @tanstack/react-query / openapi-fetch / vite / vitest /
   eslint / typescript ＋ `package.json` の `overrides` による脆弱性パッチ（`js-yaml`・`brace-expansion`）が消える。
3. **プロセスが 1 本に**。現在は api-server ＋ vite dev server の 2 本立ち上げが検証手順に入る
   （`web/vite.config.ts` の proxy 先は既定 `http://localhost:8080`。`PADDOCK_API_TARGET` はポート競合時の任意上書き）。
4. **CI の SPA 依存の消滅**。`.github/workflows/ci.yml` の `web` ジョブ 1 本（全 8 ステップ。うち検査は
   typecheck / eslint / vitest / `gen:api` ドリフト検証 / `vite build` の 5 で、残りは checkout / setup-node / `npm ci`）と、
   `docker-build` matrix の `deployments/web.Dockerfile`（`node:22-slim` で `npm ci` + `npm run build`）が不要になる。

##### 見送る理由

1. **0.5.0・アナウンスから 8 日・breaking changes 明言**。ライブ盤は実際に賭ける判断に使う画面であり、
   2 週間で 13 版動いている 0.x に載せるのは順序が逆。
2. **Topcoat の弱点が paddock の要求とど真ん中で衝突する**。Topcoat 自身がクライアント反応性を
   early stages と認めているが、paddock のライブ盤はまさにそこ——オッズ追従のポーリング
   （`web/src/routes/RaceBoard.tsx` の未発走ゲート付き `refetchInterval`・`web/src/routes/RaceList.tsx` の
   predict-watch スイープ追従）、`RaceBoard` / `ExecutionPanel` の対話編集（賭け金・払戻の手入力）、
   `RaceList` のソート・フィルタ（`SortTh` / `FilterChip`）。HTMX へフォールバックして書き直す価値は薄い。
   （なお `web/src/lib/useResultsRefresh.ts` はオッズではなく `POST /api/results/{date}:refresh` の
   着順取り込み／自動精算ポーリングであり、これも SSR 化すると作り直しになる。）
3. **DB 直読み構成を採ると OpenAPI 一級成果物の方針と衝突する**（＝理由 4 後段とは排他の分岐で、
   こちらは api-server を廃止する側）。`src/interface/rest-controller`
   （.rs 2,730 LOC。この LOC には同 crate の `src/openapi.rs` も含む）と、`src/apps/api-server/tests/openapi.rs` /
   `openapi_route_parity.rs` の契約テスト 2 本は、utoipa コードファーストで API 契約を担保するための投資
   （方針の出典は ADR 0022）。**SSR コンポーネントが DB を直読みする構成では
   この契約自体が消える**。SPA を捨てるだけでなく actix-web + utoipa の資産を捨てる判断になる。
   （Topcoat のロードマップには `OpenAPI` endpoints があるため、実装されればこの理由は弱まる。現状は未実装。）
4. **推奨形のルーティングがレイヤ構成と当たり、HTTP スタックが 2 本になる**。paddock は
   `src/` 直下を domain / use-case / interface / infrastructure / apps の 5 層に分けており
   （ADR 0064 の「rest-controller・use-case・rdb-gateway・api-server の 4 層」は read API 1 本が貫く
   crate の列挙で、この 5 層とは別の切り口）、
   Topcoat の推奨形 `module_router!` は「app モジュール木＝URL 木」を要求する
   （明示パス属性で回避はできるが、その場合は推奨形から外れる）。
   加えて **api-server を残す分岐（＝理由 3 とは排他）では HTTP スタックを 2 本抱える**。Topcoat は自前ルータを持ち、
   optional な `tower` feature で組み込めるのは tower service（axum router 等）であって
   **actix-web はこれに該当しないため、feature では 1 本に畳めない**。
5. **Tailwind 前提の同梱 UI が旨味にならない**。paddock の web は `web/src/styles.css` 1 枚の手書き
   ダークライブ盤で Tailwind を使っていない。同梱の shadcn/ui 風コンポーネント群は活かせないので、
   **"batteries-included" の売りのうちこの分は利点 0**。ただし `tailwind` feature は default 外なので、
   切っておけばコスト増にはならない——これは減点ではなく「移行の動機が 1 つ減る」という意味に留める。
6. **移行の実利が小さい——フロントは薄い**。`web/src/lib/bets.ts` は API が返す `RecommendationBet`
   （＝Rust `build_portfolio` の出力）に UI 編集と 100 円単位ガードを重ねる純関数層であり、
   **買い方ロジックの second source にはなっていない**（詳細は下記「関連」）。
   ただし**ルール由来の定数・判定が TS 側に少量ある**のは事実で、ここは正確に記録しておく:
   `web/src/lib/live.ts` の `DANZEN_WIN_ODDS_MAX = 1.9`（CLAUDE.md「断然人気は EV がマイナスになりがち」由来）と
   それを使う `skipReason()`、`SOON_MINUTES = 20` / `STALE_MINUTES = 10`（predict-watch の窓 40 分・間隔 5 分由来）、
   `web/src/lib/board.ts` の `DEFAULT_RACE_BUDGET = 5000`、`web/src/lib/constants.ts` の
   `DEFAULT_SESSION_BUDGET` / ポーリング間隔。**これらは表示用の閾値・既定値であって配分・混戦判定・
   組み合わせ生成のロジックではない**ため、Rust 回収の価値は「あるが小さい」。フレームワーク移行を
   正当化する規模の移行動機ではない、が正確な言い方。

#### 却下した代替案

- **中間案: Topcoat の SSR から既存 REST API を叩き、OpenAPI 契約を保ったまま SPA だけ置き換える**。
  見送り理由 3（契約の消滅）は回避でき、**利点 2・4（Node 依存木・CI の SPA 依存）は取れる**。
  一方で **利点 3（プロセス 1 本化）は原理的に得られない**——Topcoat サーバと actix-web の api-server で
  dev / prod とも 2 プロセスが残る。そして **最大の旨味である利点 1（型境界の消滅）は部分的にしか得られない**：
  両端が Rust なので `src/interface/rest-controller/src/schema/*`（utoipa の Rust 型）を crate 依存で
  共有すれば **codegen（`openapi-typescript` → `schema.d.ts`）自体は消せる**。ただし
  **in-process 呼び出しでない限り HTTP シリアライズ境界と 2 プロセスの運用は残る**ため、
  利点 1 の本体である「DB〜画面まで単一の型検査」には届かない。
  対価として 0.x の breaking change・HTTP スタック 2 本・クライアント反応性の弱さ
  （見送り理由 1・2・4）は全部残る。費用対効果が逆なので却下。
- **段階移行（Topcoat と SPA を並走させ画面単位で移す）**。2 つ目の HTTP スタック・2 系統のスタイル体系・
  2 系統のテスト基盤を維持期間中ずっと抱えることになり、「一時的な修正をしない」に反する。
  0.x の breaking change を並走期間中に被り続けるのも悪い。
- **Python 部分（`scripts/predict-check/`・`tools/mdq/`）の Rust 化を Topcoat 起点で進める**。
  これらは Web でなく Topcoat と無関係。Rust 化の是非は独立した論点であり、本 ADR で混ぜない。

#### 再評価の条件（reject-for-now）

**再評価の起点になるのは 1 または 2 のいずれか**（3 は単独では起点にせず、1 / 2 と併せて見る補助的観測点）。
それまでは棄却を維持する。

1. **Topcoat が 1.0 に到達**（breaking change の頻度が収まる）。
2. **クライアント反応性が SPA 相当になる**。観測点はロードマップの
   **(More) reactivity（`topcoat-runtime`）と Islands**、および Client-side navigation + prefetching。
   対話編集・ポーリング更新・表のソート/フィルタを HTMX フォールバック無しで素直に書けるかで判定する。
3. （補助的な観測点）**ロードマップの `OpenAPI` endpoints が実装される**。見送り理由 3 が弱まるため、
   1 または 2 と併せて再評価の後押しになる。

再評価時に PoC を挟むなら、**既存 SPA・api-server を不変のまま、read-only かつ対話性がほぼ無い
新規画面 1 枚だけ**（例: 回収率レポート）を Topcoat 単体で作る。既存画面の書き換えから始めない。
実測する項目:

- ランタイムに Node なしで動くか。
- domain / use-case レイヤをまたいで呼べるか（推奨形 `module_router!` と両立するか）。
- **`tailwind` feature を切ったまま成立するか**（paddock は手書き CSS なのでこれが既定路線）。
  使う判断になった場合のみ、**ビルド時の Tailwind CLI をピン供給できるか**を確認する
  （`BuildConfig::executable()` で GitHub からのダウンロードを止め、外部 action を SHA ピンする CI 規律と揃えられるか）。

#### 影響

- **コード変更なし**。`web/`・api-server・rest-controller・`package.json`・CI いずれも不変。
- 本 ADR は決定記録のみ。CLAUDE.md・買い方ルール・本番定数への影響なし。

#### 関連

- 出自: セッション中の自主評価（対象 Issue なし）。
- 出典（0.x で docs が動くため GitHub 上のものは commit `a62195b` にピンする）:
  [Announcing Topcoat（tokio.rs, 2026-07-22）](https://tokio.rs/blog/2026-07-22-announcing-topcoat)、
  [README](https://github.com/tokio-rs/topcoat/blob/a62195b6daea807cc9728ad800529b0aaa418b33/README.md)、
  [router.md](https://github.com/tokio-rs/topcoat/blob/a62195b6daea807cc9728ad800529b0aaa418b33/crates/topcoat/docs/router.md)、
  [tailwind.md](https://github.com/tokio-rs/topcoat/blob/a62195b6daea807cc9728ad800529b0aaa418b33/crates/topcoat/docs/tailwind.md)、
  [getting_started.md](https://github.com/tokio-rs/topcoat/blob/a62195b6daea807cc9728ad800529b0aaa418b33/crates/topcoat/docs/getting_started.md)、
  [crates.io: topcoat](https://crates.io/crates/topcoat)、[HN 議論](https://news.ycombinator.com/item?id=48952067)。
  **crates.io 由来の数値（0.5.0 / 2026-07-27・累計 DL 2,466・13 版）と HN スレッドはピンできないため、
  評価基準日 2026-07-30 のスナップショットであり後から厳密には再現できない**（版一覧は crates.io API で再取得可）。
- 置き換え対象の仕様: [docs/specifications/web-spa.md](../specifications/web-spa.md)（`status: Confirmed`）、
  ADR 0069（iCloud 書き出しを全廃し閲覧を REST API + SPA に一本化）。
  **なお web-spa.md の鮮度方針は「既定は自動ポーリングしない／恒常的な全画面ポーリングはやらない。例外は
  `results:refresh`（#381・ADR 0068）だけ」となっており、実装済みの RaceBoard（#475）・RaceList（#372）の
  オッズ追従ポーリングを反映していない（spec が stale ＝ CLAUDE.md の `Conflict` 相当）**。本 ADR の
  見送り理由 2 は実装側を事実として採っている。この spec 更新は本 ADR のスコープ外なので
  **追跡 Issue [#567](https://github.com/taito-station/paddock/issues/567) で解消する**。
- API 契約の方針: ADR 0022（OpenAPI を一級成果物とし、utoipa コードファースト＋
  `docs/api/openapi.json` のスナップショット検証で担保する決定）。実装は `src/interface/rest-controller/`。
- 買い方ロジックの二重実装: ADR 0064 は**当時の正本を `live_ev.py` に一本化**し、
  「Rust domain や TS に再実装すると second source が生まれる」と警告した。その後 #346 で writer が
  Rust `predict-watch` / `build_portfolio` に一本化され、**正本が Rust 側へ移って `live_ev.py` が
  second source として残った**（現行の CLAUDE.md「予算・配分」の記述がこれ）。本 ADR は、`web/src/lib/bets.ts` が
  この二重実装に**当たらない**（配分・混戦判定・組み合わせ生成を持たず、`RecommendationBet` への
  UI 編集と 100 円単位ガードのみ）ことを確認した記録も兼ねる。
- 同型のステータス運用の先例: ADR 0067（棄却（reject-for-now）＋再検証の条件）。
