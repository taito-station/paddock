# 0054. fractional Kelly 配分による買い方最適化の棄却（#316 / #272 配下）

## ステータス

棄却（production の配分は現行ヒューリスティック＝最大剰余法・固定予算・最低¥100 を維持。
CLAUDE.md「買い方ルール」不変）

## コンテキスト

#272（予測フロー再設計）配下で、確率モデルのレバーが枯渇した（ADR 0052/0053）後に残る別軸
「賭け方（staking）」を検証した。現行の買い目配分は経験則（券種固定予算・確率重み・最低¥100 確保の
最大剰余法）で、確率とオッズに対する理論最適配分（fractional Kelly）になっていない。#316 は
「同じ確率でも配分を Kelly 最適化すれば長期 ROI を底上げできる」仮説を、リーク無し walk-forward で
現行ヒューリスティックと比較し、採否を本 ADR に残す（#309/0052/0053 と同じ棄却規律）。

Kelly 式は本番 Rust `src/domain/src/betting/kelly.rs`（ADR 0003、net odds `b=gross-1`,
`f=(p·b−q)/b`, clamp）に実装済みだが、**本番の買い目配分では未使用**（live_ev.py / predict-check は
ヒューリスティック）。本 ADR はこの Kelly を実際の配分に使った場合の損益を測る。

## 検証（71R / `/tmp/bt252` walk-forward・`scripts/predict-check/kelly_compare.py`）

候補ユニバースを現行 baseline_pf（馬連◎軸ながし top5 + 三連複◎軸ながし top5）に固定し、配分方式だけを
変えて比較した。EV/Kelly 判定は DB 盤面オッズ、清算は result.html 実配当で分離（循環回避、umaren_backtest と同方針）。

**(A) 定額土俵**（総賭金を現行と同額 ¥3,500/R に固定し、券種内の重みだけ確率重み→Kelly 比率に置換）:

| 配分方式 | 実ROI | 的中 | σ(per-race ROI) |
|---|---|---|---|
| 現行（確率重み） | **75.5%** | 48% | **92.5** |
| Kelly 重み | 65.9% | 48% | 123.8 |

**(B) bankroll 土俵**（開始 ¥100,000・71R 時系列・実払戻清算。現行=固定¥3,500/R flat 張り vs
fractional Kelly λ=λ·Σf_leg·bankroll）:

| 戦略 | 最終資金倍率 | 実ROI(per-yen) | 的中 | maxDD | 近似破産率 |
|---|---|---|---|---|---|
| 現行（flat ¥3,500） | 0.39x | 75.5% | 48% | 61.1% | 1.5% |
| Kelly λ=0.25 | 0.57x | 40.6% | 9% | 46.9% | 0.0% |
| Kelly λ=0.5 | 0.33x | 39.9% | 10% | 71.4% | 0.1% |
| Kelly λ=1（full） | 0.08x | 31.8% | 11% | 93.9% | **100%** |

（破産率＝レース順 2000 本ブートストラップで残高が開始×0.2 以下に到達した経路の割合。）

**観察**:

- **定額土俵では Kelly 重みが現行に劣る**（ROI 65.9% < 75.5%、σ も 123.8 > 92.5 と悪化）。同一総額・同一
  候補・同一オッズで「どの脚に厚く張るか」だけを変えた apples-to-apples 比較で、Kelly 重みは回収率・分散とも負け。
- **full Kelly（λ=1）はほぼ確実に破産**（破産率 100%・最終 0.08x・maxDD 93.9%）。issue 補足「確率推定に
  系統誤差があると過剰張りで破産を早める」を実証。モデルの favorite-longshot ミスキャリブレーション
  （#246・ADR 0047/0050）が Kelly の edge 推定を歪め、過大張りに直結する。
- **fractional（λ=0.25）の最終資金 0.57x > flat 0.39x は配分改善ではない**。per-yen ROI は 40.6% と
  flat 75.5% より**悪く**、的中も 9%。これは Kelly が +EV と判定した脚だけを bankroll 比で**極小額**張る
  ため、控除率（馬連~20%・三連複~27.5%）による資金流出が単に遅いだけ＝実質「ほとんど見送り」。edge を
  取って増やしたのではなく「賭ける額を絞って損を遅らせた」結果で、value 捕捉の証拠ではない。

## 決定

**fractional Kelly による配分最適化は棄却**する。production は現行ヒューリスティック配分（券種固定予算・
確率重み・最低¥100 の最大剰余法、CLAUDE.md「買い方ルール」）を維持し、変更しない。Rust `betting/kelly.rs`
は EV 候補選抜のための curation（min_kelly）用途に留め、賭け額配分には用いない（現状どおり）。

## 理由

- 同一土俵（定額）で Kelly 重みは現行に ROI・分散とも劣る。Kelly が理論最適となる前提「確率推定が正しい」が
  本 PJ では成立せず（モデルは単勝市場に勝てない＝ADR 0053、かつ favorite-longshot 較正不良）、誤った確率に
  Kelly を適用すると過大張り（full で破産）か過小張り（fractional で実質見送り）に振れるだけで、配分の質は上がらない。
- bankroll 土俵で λ=0.25 が資金を残すのは「賭けない方向に縮む」効果で、これは既に CLAUDE.md の
  「ROI≥100% のレースだけ張る／−EV は見送る」ゲートが担っている規律と同じ。Kelly を新たに入れる便益がない。
- 既存の配分棄却履歴（相手拡大=105R 悪化 / 配分 floor 除去=71R 悪化、`betting-rule-history.md`・memory
  `project_alloc_floor_finding`）と整合的に、「経験則配分を理論配分で置換しても回収率は改善しない」を再確認した。

## 影響 / 留保

- production・既存挙動とも不変。本 ADR は「現行配分維持」の根拠を数値で固定する記録。
- **リーク（common-mode）**: bt_pred の確率は `analyze predict` 由来で過去走較正にリークの可能性
  （memory `alpha_sign_and_predict_leak`）。ただし全配分方式が同一 probs を共有するため、相対比較
  （Kelly vs 現行）にはキャンセルして影響しない。確率の絶対精度ではなく配分方式の優劣を測る本 ADR では妥当。
- **単一窓・71R（約 8 開催日）で検出力は限定的**。ただし (1) 定額土俵で Kelly が現行に明確に劣り、(2) full
  Kelly が破産する向きは確率較正不良から理論的にも予期され、(3) 既存の配分棄却知見と整合するため、棄却の
  向きは頑健と判断する。確定的な配分再設計には複数窓を要するが、本 ADR の主張は「Kelly 置換の棄却」に限定。
- **ゲートの注記**: bankroll 土俵の flat は全鞍機械買い（ROI≥100% ゲート無し）で、61% の資金流出は「毎レース
  −EV を張ると負ける」事実を反映する。実運用はゲートで −EV を見送るため flat より良い。Kelly の自己縮小も
  同じ −EV 回避に帰着し、ゲート済み運用に対する追加便益は無い。
- value シグナル自体（穴の高オッズ的中由来の ROI）の真偽は #305/#314 系に委ね、本 ADR は配分方式の優劣に限定。

## 再現方法

```sh
# 入力 /tmp/bt252 は #252 手順（gen_win_backtest_data.sh + bt_exotic_odds.tsv の DB エクスポート）で生成。
python3 scripts/predict-check/kelly_compare.py --bt-dir /tmp/bt252
# (A) 定額土俵で現行(確率重み) vs Kelly重みの ROI/σ、(B) bankroll 土俵で flat vs Kelly λ掃引の
# 最終資金倍率・per-yen ROI・maxDD・近似破産率を比較する。full Kelly(λ=1) の破産率 100% を確認。
# 単体テスト: python3 -m pytest scripts/predict-check/test_kelly_compare.py（Kelly 式の Rust 鏡映・丸め・sim）。
```

## 関連

- Issue: #316（Kelly/ポートフォリオ最適配分・本 ADR）/ #272（予測フロー再設計・親）/ #314（エキゾ・ミスプライス）/
  #315（オッズの動き）/ #246（win 較正）/ #305（純モデル value 検証）
- ADR 0003（EV・Kelly 配分の Rust 実装）/ ADR 0052・0053（α blend・学習モデルの棄却＝確率レバー枯渇）/
  ADR 0047・0050（favorite-longshot 較正不良）
- `docs/specifications/betting-rule-history.md`（配分棄却履歴）/ memory `feedback_betting_staking`・
  `project_alloc_floor_finding`
