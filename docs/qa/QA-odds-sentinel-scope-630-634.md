# QA — 番兵判定に券種を持ち込むか（#630・#634）

一次資料: [#630](https://github.com/taito-station/paddock/issues/630) /
[#634](https://github.com/taito-station/paddock/issues/634)（転記しない・ADR 0074）。
本文は `gh issue view 630` / `gh issue view 634` で取得する。

## Q0: フラット判定の誤爆は実在するリスクか（実装前の実測）

- 観測/根拠: 共有 DB を値で集計した（2026-08-18）。

  | 項目 | 実測 |
  |---|---|
  | `9999.9` ちょうどの行 | `race_odds` / `race_odds_snapshots` とも **0 行** |
  | 9000〜11000 帯の正当配当 | trio **6,244 行** / trifecta **56,230 行** / exacta 699 行 / quinella 48 行 |
  | `99999.9` の行 | quinella 156 / exacta 859 / trio 1,599（いずれも番兵） |

- 回答: **確定。リスク帯は恒常的に実在する**。`9999.9` ちょうどが今日 0 行なのは偶然で、trio/trifecta
  はその前後の帯に数千〜数万行が常在する。出れば `debug` ログで黙って消える（観測不能）ため、
  「まだ出ていない」は安全の根拠にならない。
- 反映先: ADR 0088 / `docs/specifications/netkeiba-datasource.md`

## Q1: 単勝・複勝に番兵はあるか（#634）

- 観測/根拠: 両テーブルで `odds IN (9999.9, 99999.9, 999999.9) AND bet_type IN ('win','place')` が
  **0 行**。win/place は type=1 応答（`fetch_win_place_odds`）由来で、未発売の馬は行ごと欠ける形で
  観測されており、番兵値の観測が無い。
- 回答: **確定。win / place に番兵は無い**。正本ファイルに行を置かない（空タプルでなく「行なし」）。
  「win に番兵は無い」（値の事実・`False`）と「win という券種は無い」（ラベル誤り・`ValueError`）を
  区別する。将来 netkeiba が win に番兵を入れ始めたら正本ファイルに 1 行足すだけで両言語に効く。
- 反映先: ADR 0088 / `src/domain/src/odds/netkeiba_sentinels.txt`

## Q2: `TryFrom<f64>` を残して券種付きを追加する折衷はなぜ採らないか

- 観測/根拠: #621 の本質は「番兵をどの層も見ていなかった」＝ガードを通らない経路が静かに存在した
  こと。券種なし版が残ると、新規コードがそれを呼んでも型検査は通り、フラット判定（誤爆）が
  静かに復活する。既定値付き Python API も同型（更新漏れが素通り）。
- 回答: **確定。置換のみ（削除 + タプル版）**。渡し忘れをコンパイルエラー／`ValueError` に変える
  ことが唯一の構造的防御。タプル入力の前例は `TryFrom<(OddsValue, OddsValue)> for PlaceOdds`。
- 反映先: ADR 0088 / `odds_value.rs` / `odds_guard.py`

## Q3: 既存の呼び出し口で券種は手に入るか

- 観測/根拠: 本番 3 か所を実査——`assemble_netkeiba`（7 つの独立ループ＝静的に確定）/
  `save_race_odds::classify_row`（呼び出し元が `row.bet_type: String` を保持）/
  `find_race_odds::parse_odds_value・parse_band`（`rows_to_race_odds` の `match bet_type` の中）。
  Python の差し込みは 5 スクリプト・8 か所（`fetch_wide.py`×1 / `live_ev.py`×2 /
  `umaren_backtest.py`×1 / `snapshot_ev_report.py`×2 / `gate_calibration.py`×2）で、
  すべて呼び出し文脈に券種がある。
- 回答: **確定。全箇所で追加取得なしに券種を渡せる**。`ingest.rs` の生 f64 経路は保存境界の
  `classify_row` が券種付きになることで自動的に券種別になるため変更不要。スクレイパ側
  `parse/odds.rs` には足さない（ADR 0086 決定 2 の維持）。
- 反映先: 実装（PR #630/#634 対応 PR）

## Q4: `find_race_odds_morning`（朝時点復元）に挙動差は出るか

- 観測/根拠: 朝時点の候補選定は `bet_type='trio'` の DISTINCT `fetched_at`。trio の番兵は
  `99999.9` で券種別化後も引き続き弾かれる。trio の `9999.9` は新たに読めるようになるが、
  現時点の DB に該当行は 0（Q0）。
- 回答: **確定。挙動不変**。回帰テストは #632/#633（未発売の観測記録）側で張る（同じファイルを
  触る PR-D と分離するため）。
- 反映先: ADR 0088「影響」
