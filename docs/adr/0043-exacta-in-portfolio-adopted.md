# 0043. 買い目ポートフォリオへの馬単（exacta）導入

## ステータス
採用

## コンテキスト

predict 本番の買い目生成器 `build_portfolio`（`src/domain/src/portfolio/mod.rs`）は、軸＝本命を
相手に流す「馬連＋ワイド＋三連複」だけを生成し、**馬単（exacta）を候補に出していなかった**（#246）。

Harville モデルは IIA 的性質から人気薄馬の「1着」確率を過大評価しがちで、その結果、穴を絡めた
**馬連（着順不問）の EV が馬単（着順固定）より高く**算出される傾向がある。しかし実戦では「人気薄は
2〜3着はあっても1着は薄い」ケースが多く、本命→穴の馬単に切り替えると配当プレミアムが取れる局面がある
（動機: 2026-06-27 福島R7。馬連 192.1 倍 → 馬単 253.6 倍）。馬単を候補に出せないことでこのエッジを
取りこぼしていた。

下地はすでに揃っていた: `BetCombination::Exacta(OrderedPair)`、`RaceOdds.exacta`（odds scraper /
DB gateway とも populate 済み）、`harville_exacta`、predict の馬単表示（`format_combination`）。

## 決定

`build_portfolio` の「馬連バケット」を**連系ペアバケット**と再解釈し、軸-相手の各ペアで
`Quinella(軸,相手)` と `Exacta(軸→相手)` の実効 EV（`leg_ev` = 的中確率 × オッズ、収支シミュレータ
`simulate` 委譲）を比較して、**EV が高い方を 1 脚だけ採用する**（`pick_pair_leg`）。

- 採用は **両券種のオッズが揃い、馬単が strict に馬連を上回るときだけ**。tie・どちらか欠落は着順不問の
  馬連を維持する（"常に馬単優先とはしない" の担保。issue 補足の方針）。
- 穴の 1 着確率が縮約されるほど `harville_exacta(軸→穴)` が相対的に有利化するため、この選択は #246(A)
  の win_prob 冪変換（ADR 0042）と連動して機能する。

### 4要素 alloc 化を退けた理由

`PortfolioConfig.alloc:(u32,u32,u32)` を `(馬連, ワイド, 三連複)` から4要素化して馬単を独立バケットに
する案は採らない。理由:

- 馬連と馬単を**並列に**買うと 1 レースの券種が増え、固定予算（既定 ¥5,000）が薄まる。issue の意図は
  並列追加ではなく「同組合せの馬連より優位なら馬単に**置き換える**」。
- 既存テスト（`alloc:(1,1,1)` リテラル多数）と `session.rs` の `PortfolioConfig::default()` 利用を
  壊さずに済む。第1要素を「連系ペア」と読み替えるだけで予算配分・点数・100 円単位ロジックは無改修。

## CLAUDE.md 買い方ルールとの関係

CLAUDE.md は 3 券種（ワイド/馬連/三連複）を既定とし、馬連を「◎が飛んでも拾える着順不問の保険」と
位置づける。本変更は馬連を**ペア単位で馬単に置換しうる**ため、置換されたペアでは着順不問の保険性が
一部弱まる（issue 承知のトレードオフ）。これを次で緩和している:

- 置換は EV が strict に優位なペアのみ。tie・欠落は馬連維持。
- 三連複ながし・ワイドの保険脚は不変。
- 馬単化は (A) の校正で「穴は1着になりにくい」と根拠づけられたときに発火する設計（恣意的な馬単優先を
  しない）。

将来、置換ではなく併買や発火条件の調整を検討する余地は残す。

## 影響

- `build_portfolio`: ペア脚生成を `pick_pair_leg` 経由に変更。`PortfolioConfig.alloc` の第1要素は
  「連系ペア（馬連/馬単のうち EV 優位な方）」の意味になる（型・既定値は不変、後方互換）。
- 既存 `build_portfolio` テストは `sample()` が exacta オッズを持たないため全 quinella 採用で無改修通過。
  馬単 swap の新規テストを追加。
- 診断のため `pair_ev_diagnostics`（馬連 vs 馬単 両方向 EV）も同 module に追加（#246-C, ADR 不要の表示）。

## 再現・検証

```sh
cargo test -p paddock-domain portfolio
# exacta_chosen_when_ev_beats_quinella / quinella_kept_when_exacta_lower_ev /
# exacta_swap_preserves_budget_and_units で swap 挙動と予算・100 円単位の維持を確認。
```
