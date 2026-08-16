# QA — netkeiba の未発売番兵がオッズとして EV に食われる（#621）

一次資料: [#621](https://github.com/taito-station/paddock/issues/621)（転記しない・ADR 0074）。
本文は `gh issue view 621` で取得する。

## Q0: 番兵値は `99999.9` だけか（実装前の実測）

- 観測/根拠: 共有 DB の `race_odds` を値で集計した。

  | 券種 | 番兵値 | 行数 | レース数 |
  |---|---|---|---|
  | 三連単 | **`999999.9`** | 32,973 | 227 |
  | 馬連 / 馬単 / 三連複 | `99999.9` | 2,614 + α | 80+ |
  | ワイド | `9999.9` | **0**（保存前に落ちている） | — |

- 回答: **確定。番兵は 2 種類ある**（issue は `99999.9` のみを挙げていた）。三連単の `999999.9` は
  買い目には使わないが、`race_odds_snapshots` を読む分析経路（`gate_calibration` 等）には効く。
  ワイドの `9999.9` は DB に 1 行も無い——**弾かれていたのは番兵だからではなく、相方の
  `odds_high=0.0` が下限違反だったから**（後述 Q2）。
- 反映先: ADR / docs/specifications/netkeiba-datasource.md

## Q1: プレースホルダの判定方法は（上限か特定値か）

- 観測/根拠: `odds >= 1000` の行を実測すると、三連単に `111971.9` / `200886.6` / `87446.6` など
  **1 レースにしか出ない大きな値が多数**あり、これらは正当な高配当（3連単は 10 万倍超が普通に出る）。
  一方 `99999.9` / `999999.9` は多数のレースに繰り返し現れ、同一レース内の正常値の最大
  （例: 2,083.5）と桁が断絶している。
- 回答: **確定。特定値の除外**（ユーザー確認済み）。上限方式は正当な大穴を殺す。番兵は netkeiba が
  入れる固定値なので、値を名指しする方が誤爆しない。比較は epsilon（`1e-6`）で行う。
- 反映先: ADR / `src/domain/src/odds/odds_value.rs`

## Q2: なぜワイドだけ守られていたのか（band と scalar の非対称の実体）

- 観測/根拠: netkeiba のワイド番兵は `["9999.9","0.0","--"]` で **両端がパースできる**ため
  `parse_wide_odds` は通る。落ちているのは `assemble_netkeiba` / `save_race_odds` で
  `OddsValue::try_from(0.0)` が下限違反になるから。スカラー券種は `odds_high` が無いので
  この判定に掛からず素通りしていた。
- 回答: **確定。ダミー検知ではなく偶然**。番兵そのものはどの層でも見ていなかった。
  `OddsValue` に番兵判定を入れれば、band / scalar の区別なく同じ基準になる。
- 反映先: ADR

## Q3: 修正をどの層に入れるか

- 観測/根拠: `save_race_odds::is_invalid_odds` と `find_race_odds::parse_odds_value` は
  **両方 `OddsValue::try_from` に委譲済み**（値域条件を複製しない設計）。スクレイプ経路の
  `assemble_netkeiba` も同じ変換を通る。
- 回答: **確定。`OddsValue::try_from` 一点**（ユーザー確認済み）。保存・読み出し・組み立ての
  全経路に一撃で効き、**既に DB にある番兵行も読み出し時に無害化**される。
  スクレイパ側にも足すと値域判定が 2 か所になる（ADR 0064 の second source）ので入れない。
- 反映先: ADR / `src/domain/src/odds/odds_value.rs`

## Q4: 既に DB に入っている行をどうするか

- 観測/根拠: `race_odds` に 1,599 行（trio）+ 156 行（quinella）、`race_odds_snapshots` に
  185,794 行。snapshots は 15 分毎の live オッズを積んだ**再取得不能資産**（#232/#492）。
- 回答: **確定。DELETE しない**（ユーザー確認済み）。番兵は「その時点で未発売だった」という
  事実の記録でもある。読み出し側で無害化し、**Python の分析経路にも同じ除外を入れる**
  （`scripts/predict-check/odds_guard.py`）。
- 反映先: ADR

## Q5: EV 側にフィルタを足す必要はあるか

- 観測/根拠: `leg_metrics`（`src/domain/src/portfolio/mod.rs`）は `ev = hit_prob * odds`。
  的中確率はダミー倍率 1.0 で別計算なので**正常**、EV だけが壊れる。オッズが無い脚は
  `bet.odds = None` になり、`format_portfolio` の「オッズ未取得」アームと `build_portfolio` の
  priced フィルタで**既に扱われている**。
- 回答: **確定。EV 側は触らない**。読み出しで弾けば既存の「オッズ不明」経路に自然に落ちる。
- 反映先: ADR

## Q6: ADR 0076 / 0079 の測定母集団は汚染されていたか

- 観測/根拠: `race_odds_snapshots` の番兵行は **trio 7,259 行 / 111 レース**、
  quinella 654 行 / 47 レース。snapshots を持つ全レースは 486 なので、**約 23% のレースが該当**。
  `gate_calibration.py` が出す 2 つの ROI を追うと、汚染の効き方が別々だった:

  - **判定 ROI（`judged_roi`）**: `races.append({... "judged_roi": row["roi"] ...})` の `row` は
    `live_ev_snapshots` の行で、`roi` 列は **predict-watch が Rust で計算して保存した値**
    （`snapshot.rs:81` の `ev.roi * 100.0`）。番兵が `EV = 的中確率 × オッズ` を 3 桁にする経路は
    **ここだけ**で、保存済みの数値なので本 PR の修正でも `odds_guard` でも直らない。
  - **市場整合 ROI（`market_fair_roi`）**: `q = (1/o)·W/inv_sum` に `exp += amount·q·o` なので
    **`o` が約分され `amount·W/inv_sum`** になる。番兵脚は「ゼロ寄与」ではなく他脚と同額寄与し、
    影響は `inv_sum` に `1/999999.9 ≈ 1e-6` が乗るぶんだけ（しかも全脚の `q` を下げる過小評価方向）。

- 回答: **確定。汚染は判定 ROI 側にあり、市場整合 ROI はほぼ無影響**。
  したがって**再測定は「`odds_guard` を入れて計算し直す」では済まない**——判定 ROI は番兵除去後の
  predict-watch が新しく記録し直す必要があり、既存の `live_ev_snapshots` 行は使えない。
  ADR 0076 が「ROI ≥ 100% は 0 件」と結論したこと自体は、その期間の買い目の脚に番兵がほぼ
  乗らなかったことを示唆する（買い目は model top5 ＝人気上位、番兵は売れていない人気薄に出る。
  本 PR の実地確認でも買い目に乗った番兵は 0 点）。ただし断定はできないので取り直す。
- 反映先: ADR / #625
