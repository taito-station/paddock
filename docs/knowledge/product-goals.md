---
status: Confirmed
kind: knowledge
doc_class: [D01]
tags: [D01]
sources:
  - docs/original-docs/0027-accuracy-lever-is-market-blend-not-data-volume.md
  - docs/original-docs/0028-konsen-odds-trigger-rejected.md
  - docs/original-docs/0030-konsen-trio-partner-width-rejected.md
  - docs/original-docs/0033-conditional-win-bet-rejected.md
  - docs/original-docs/0034-alpha-retune-recency-rejected.md
  - docs/original-docs/0035-recent-form-weight-retune-rejected.md
  - docs/original-docs/0036-recent-form-trend-n-rejected.md
  - docs/original-docs/0037-place-show-exotic-market-blend-rejected.md
  - docs/original-docs/0038-jockey-recent-form-rejected.md
  - docs/original-docs/0039-formation-2axis-rejected.md
  - docs/original-docs/0040-ev-gate-threshold-lowering-rejected.md
  - docs/original-docs/0041-umaren-only-strategy-rejected.md
  - docs/original-docs/0043-exacta-in-portfolio-rejected.md
  - docs/original-docs/0046-allocation-prob-weight-no-floor-rejected.md
  - docs/original-docs/0047-place-show-power-decompression-adopted.md
  - docs/original-docs/0050-placeshow-raw-score-retune-rejected.md
  - docs/original-docs/0052-alpha-blend-removal-rejected.md
  - docs/original-docs/0053-learned-fundamental-model-rejected.md
  - docs/original-docs/0054-kelly-staking-rejected.md
  - docs/original-docs/0055-ev-layer-separation-circular-break.md
  - docs/original-docs/0058-pedigree-sire-feature-rejected.md
  - docs/original-docs/0059-market-calibration-correction-rejected.md
  - docs/original-docs/0060-betting-axis-lock-preclose-topup.md
  - docs/original-docs/0061-running-style-feature-rejected.md
  - docs/original-docs/0062-workout-cyokyo-feature-rejected.md
  - docs/original-docs/0063-exotic-mispricing-harvest-rejected.md
  - docs/original-docs/0064-live-ev-buy-view.md
  - docs/original-docs/0067-late-money-odds-drift-signal-rejected.md
  - docs/original-docs/0069-drop-icloud-writes-browser-only-viewing.md
  - docs/original-docs/0071-topcoat-framework-evaluation-rejected.md
  - docs/original-docs/0073-adr-into-original-docs-and-doc-classes.md
  - docs/original-docs/0076-roi-gate-uncalibrated-under-ev-layer-separation.md
  - docs/original-docs/0078-pin-bet-selection-across-sweeps.md
  - docs/original-docs/0079-roi-gate-display-kept-with-unreachable-note.md
  - docs/original-docs/0086-netkeiba-unpriced-sentinel-is-not-odds.md
distilled_from_sha: "b26e5cd"
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
| REQ-D01-001 | 張るレースは ROI ≥ 100% のものだけに限る。閾値は引き下げない。**ただし現行の参考ROIはこのゲートの判定指標として機能していない**（下記「ゲートの現況」）——ゲートを緩める根拠にはならないので、閾値の引き下げは引き続き行わない | ADR 0040 の再現方法（保存済み `race_odds` に `analyze predict --blend-alpha 0.2` と `scripts/predict-check/live_ev.py` を当てて全 3 券種の ROI 分布を出す。**`analyze predict` は集計統計に `as_of=None` を使うので過去レース再評価ではリークする**——ただし +EV を多く見せる向きなので「+EV 帯が薄い」という結論には保守的）を再実行し、**閾値を下げると −EV を買うことになる**ことを確認する。あわせて `scripts/predict-check/gate_calibration.py`（ADR 0076 の再現方法）で判定ROIと実現ROIの較正を測り直す。ADR 0040 時点の実測は 69R で平均 ROI 73.1% / 最高 97% / +EV 0 本、ADR 0076 の実測は 182R で判定ROI 平均 23.2% / 最高 76.8% / ゲート通過 0 本（**この測定は #621 の番兵除去前**。判定ROI の出所は `live_ev_snapshots.roi` ＝ predict-watch が保存済みの計算結果で、netkeiba の未発売番兵が EV を 3 桁にする経路はここ。**#621 の修正では直らない**ので、取り直しには番兵除去後の記録が要る（#625）。なお市場整合ROI 側は式の上で `o` が約分されるためほぼ無影響） | [ADR 0040](../original-docs/0040-ev-gate-threshold-lowering-rejected.md) / [ADR 0076](../original-docs/0076-roi-gate-uncalibrated-under-ev-layer-separation.md) / [ADR 0079](../original-docs/0079-roi-gate-display-kept-with-unreachable-note.md)（表示と運用記述） | Confirmed |
| REQ-D01-002 | 順位付けは blended 確率、EV は純モデル確率 × 市場オッズで計算する（確率と買い方の層分離） | `cargo test -p paddock-domain` の EV 層テスト（純モデル確率が EV 経路に渡ることを固定） | [ADR 0055](../original-docs/0055-ev-layer-separation-circular-break.md) | Confirmed |
| REQ-D01-003 | 軸は事前データで確定し、直前オッズでは動かさない（用途はズレ増額のみ・軸フリップ禁止）。**軸だけでなく相手・混戦判定も固定する**（実装は REQ-D23-007） | `scripts/predict-check/gate_calibration.py` の「軸（◎）の安定性」節が **`0/N`**（発走前に 2 スイープ以上あったレースで軸が入れ替わらない）。目視のログ突き合わせでは取りこぼす——ADR 0078 以前は 154R 中 軸 28R・相手 62R が黙って入れ替わっていた。機械検査は `cargo test -p paddock-domain` の `pinned_selection_survives_market_movement_while_roi_moves`（選定は不変・ROI は動く）が張る | [ADR 0060](../original-docs/0060-betting-axis-lock-preclose-topup.md) / [ADR 0078](../original-docs/0078-pin-bet-selection-across-sweeps.md) | Confirmed |
| REQ-D01-004 | ADR 0052 と同一条件（α=0.2・縮約 / 冪較正フラグなし）のトップ選好馬の単勝的中率が 28% を下回らない（890R 実測 29.9%） | `paddock-analyze backtest --from 2026-03-15 --to 2026-06-21 --blend-alpha 0.2` の `win_hit_rate`（ADR 0052 の再現方法と同じコマンド。`backtest` は m / 冪較正を既定適用しないので、本番構成で測るならフラグを明示したうえで閾値ごと測り直す） | [ADR 0052](../original-docs/0052-alpha-blend-removal-rejected.md) | Confirmed |
| REQ-D01-005 | 同条件のトップ選好馬の複勝的中率が 60% を下回らない（890R 実測 64.5%） | 同上（`show_hit_rate`） | [ADR 0052](../original-docs/0052-alpha-blend-removal-rejected.md) | Confirmed |
| REQ-D01-006 | 手動ハンデ軸精査を伴う実運用セッションの単勝的中率が、REQ-D01-004 と同条件のバックテスト水準（29.9%）を上回る＝エッジが実在することを実測で示す | 実運用セッションの `bet_records` と結果照合を 200R 以上貯め、`◎` の単勝的中率をバックテストと同じ定義で集計する（現状の観測は 1 開催日規模で母数が足りず、確定知にできる水準にない） | [ADR 0055](../original-docs/0055-ev-layer-separation-circular-break.md)（エッジ＝手動ハンデ軸精査という主張の出所） | Tentative |
| REQ-D01-007 | 買い目は「そのまま買える形」で提示する（式別 / 方式 / 軸 / 相手 / 点数 / 金額・100 円単位） | `build_portfolio` の単体テスト（配分の正）と [live-ev-buy-view.md](../specifications/live-ev-buy-view.md) の**表示形式（slip）**の契約（配分方式の正は `build_portfolio`。ライブ writer は #346 で Rust `predict-watch` に一本化済み） | [ADR 0064](../original-docs/0064-live-ev-buy-view.md) | Confirmed |
| REQ-D01-008 | 予想と買い目はブラウザから閲覧できる（ローカル完結・外部ストレージに依存しない） | `docs/api/openapi.json` のスナップショット検証（`src/apps/api-server/tests/openapi.rs`）と `web/src/lib/board.test.ts`、および [race-list-dashboard.md](../../tests/browser-test-cases/race-list-dashboard.md) の手動ブラウザ手順 | [ADR 0069](../original-docs/0069-drop-icloud-writes-browser-only-viewing.md) | Confirmed |
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
| [0034](../original-docs/0034-alpha-retune-recency-rejected.md) | recency（時間減衰）重み | Brier/LogLoss が変わらず ROI も誤差範囲。複雑性だけ増える |
| [0035](../original-docs/0035-recent-form-weight-retune-rejected.md) | `recent_form_weight` の再チューニング | 4891R で 0.25 を**有意に**上回る値が無い（最良でも LogLoss 差 0.0001〜0.0003 で過剰適合リスクに見合わない） |
| [0036](../original-docs/0036-recent-form-trend-n-rejected.md) | 直近 N 走のトレンド加重平均 | 893R で N=1（前走のみ）を上回らない |
| [0038](../original-docs/0038-jockey-recent-form-rejected.md) | 騎手の直近フォーム factor | 重み 0 が最良。機構だけ残す |
| [0050](../original-docs/0050-placeshow-raw-score-retune-rejected.md) | place/show 素スコアの m×recency×form joint retune | 脱圧縮は ADR 0047 の冪変換で足り、joint retune に上積みが無い |
| [0052](../original-docs/0052-alpha-blend-removal-rejected.md) | 市場ブレンドの廃止（純モデル化） | 純モデルは単勝 12.0% で市場 29.7% の半分以下 |
| [0053](../original-docs/0053-learned-fundamental-model-rejected.md) | 学習型モデル（条件付きロジット/PL・GBM）への置換 | モデルクラスを変えても市場を超えない |
| [0058](../original-docs/0058-pedigree-sire-feature-rejected.md) | 血統（種牡馬）適性 factor | 現行データの天井内でノイズ級 lift。**天井の主因＝factor 冗長性を確定させた ADR** |
| [0061](../original-docs/0061-running-style-feature-rejected.md) | 脚質（先行度）factor | AUC/校正は微改善するが本命指標 top1 が全 weight で劣化 |
| [0062](../original-docs/0062-workout-cyokyo-feature-rejected.md) | 調教（追い切り）評価 factor | 純モデルでは情報を持つが市場ブレンドに完全吸収される |

### B. 市場のミスプライスを突く

| ADR | 採らなかったこと | 棄却の理由（要約） |
|---|---|---|
| [0037](../original-docs/0037-place-show-exotic-market-blend-rejected.md) | place/show・exotic の市場ブレンド本番化 | 校正は微改善するが回収率が悪化。exotic は本番経路に届かない |
| [0059](../original-docs/0059-market-calibration-correction-rejected.md) | 人気-穴バイアスの較正補正 | バイアスは実在するが takeout（19%）より小さく exploitable でない |
| [0063](../original-docs/0063-exotic-mispricing-harvest-rejected.md) | エキゾ（馬連/3連複/馬単）のミスプライス収穫 | 額面 +EV は 83R・実質 5 開催日の小標本変動 |
| [0067](../original-docs/0067-late-money-odds-drift-signal-rejected.md) | late money / 単勝 log-odds drift のシグナル化 | クロージングラインが動きの情報を既に吸収済み |

### C. 買い方（券種・配分・ゲート）を広げる・複雑にする

| ADR | 採らなかったこと | 棄却の理由（要約） |
|---|---|---|
| [0028](../original-docs/0028-konsen-odds-trigger-rejected.md) | 混戦判定へのオッズ条件併用 | baseline を上回る閾値が 1 つも無い |
| [0030](../original-docs/0030-konsen-trio-partner-width-rejected.md) | 混戦時の 3 連複の相手拡大（top7・全頭） | 的中率は上がるが回収率は全 variant で低下 |
| [0033](../original-docs/0033-conditional-win-bet-rejected.md) | 条件付き単勝の追加 | 全閾値・全予算モードで 3 券種 baseline を下回る |
| [0039](../original-docs/0039-formation-2axis-rejected.md) | 上位近接時の 2 軸フォーメーション | ◎1 頭軸ながしを上回らない |
| [0040](../original-docs/0040-ev-gate-threshold-lowering-rejected.md) | EV ゲート閾値（ROI ≥ 100%）の引き下げ | ROI 100% が損益分岐そのもの。緩めると −EV を買う |
| [0041](../original-docs/0041-umaren-only-strategy-rejected.md) | 馬連特化戦略 | 3 券種ポートフォリオを上回らない |
| [0043](../original-docs/0043-exacta-in-portfolio-rejected.md) | 馬単のポートフォリオ導入 | 導入後 71R の検証で純損と確定し #271 で撤回 |
| [0046](../original-docs/0046-allocation-prob-weight-no-floor-rejected.md) | 配分の確率重み化＋脚ごと最低 ¥100 の撤廃 | 薄い脚への少額 spread が実 ROI を上げている |
| [0054](../original-docs/0054-kelly-staking-rejected.md) | fractional Kelly 配分 | 同一土俵の比較で現行ヒューリスティックを上回らない |

### D. 技術スタックの入れ替え

| ADR | 採らなかったこと | 棄却の理由（要約） |
|---|---|---|
| [0071](../original-docs/0071-topcoat-framework-evaluation-rejected.md) | `web/` React SPA の Topcoat（SSR フルスタック）置き換え | 型境界の消滅という利点はあるが、今取るには移行コストが見合わない（reject-for-now） |

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
