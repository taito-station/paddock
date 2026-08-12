# 601 — predict-watch の買い目選定がスイープ間で動く（生資料）

`predict-watch` が発走前スイープのたびに軸・相手・混戦判定を選び直している事実の実測。
issue 本文は [gh issue view 601](https://github.com/taito-station/paddock/issues/601)。

蒸留先: ADR 0078 / [product-goals.md](../knowledge/product-goals.md) REQ-D01-003 /
[ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md)。

## 母集団

`live_ev_snapshots`（`predict-watch` が各スイープで記録した軸と買い目伝票）の
**182 レース / 839 スイープ / 8 開催日（2026-07-11〜08-09）**。うち**発走前に 2 スイープ以上
あった 154 レース**が「動いたか」を判定できる母集団（1 スイープしか無いレースは比較対象が無い）。

母集団は #571（ADR 0076）の較正測定と同一。窓は EV 層分離（ADR 0055・`4b93679`）と
混戦ボックス（`c6ec8d0`）の後で、全期間が現行ロジック。

## 測定結果

| 動いたもの | レース数 | 割合 |
|---|---|---|
| 軸（◎） | 28 | 18% |
| 相手集合 | 62 | 40% |
| 混戦判定 | 3 | 2% |
| **軸は不変だが相手が動いた** | **51** | **33%** |

**終日ずっと同じ買い目だったのは 154 レース中およそ 60% しかない。**

軸が 3 種以上になったレースは 0 だが、`4→9→4→9` のような**往復**がある（最初と最後だけ見ると
同じ軸に見えるため、遷移列で確認する必要がある）。

### 軸が入れ替わったレース（抜粋）

```
2026-07-12 函館10R  軸 4→9→4→9  (ROI 28.2% → 21.8%)
2026-07-19 函館12R  軸 4→5       (ROI 28.9% → 24.1%)
2026-08-01 札幌10R  軸 1→5→1     (ROI 20.7% → 18.4%)
2026-08-01 札幌 5R  軸 4→7       (ROI 29.1% → 20.0%)
2026-08-01 札幌 7R  軸 3→6       (ROI 68.0% → 31.8%)
2026-08-02 札幌11R  軸 9→7       (ROI 28.8% → 18.3%)
2026-08-02 札幌12R  軸 14→2      (ROI 52.3% → 17.4%)
2026-08-02 札幌 5R  軸 4→2       (ROI 27.4% → 27.4%)
2026-08-02 札幌 6R  軸 3→4       (ROI 28.7% → 26.6%)
2026-08-02 札幌 8R  軸 2→14      (ROI 28.9% → 28.4%)
…ほか 18 レース
```

### 実例 1: 軸が入れ替わる（2026-08-01 札幌7R・発走 13:20）

```
captured_at(UTC) | ROI  | 軸 | 馬連の脚（組番）
2026-08-01T02:52 | 68.0 |  3 | [3,10] [3,12] [3,5] [3,6] [3,7]
2026-08-01T03:42 | 37.5 |  6 | [3,6] [5,6] [6,10] [6,12] [6,7]
2026-08-01T03:48 | 37.0 |  6 | 〃
2026-08-01T03:53 | 36.7 |  6 | 〃
2026-08-01T03:59 | 35.1 |  6 | 〃
2026-08-01T04:04 | 35.2 |  6 | 〃
2026-08-01T04:10 | 32.8 |  6 | [3,6] [6,10] [6,11] [6,12] [6,7]
2026-08-01T04:15 | 31.8 |  6 | 〃
```

朝 11:52（02:52Z）の軸は ③ だったが、12:42（03:42Z）以降は ⑥ に乗り換わっている。
さらに 13:10（04:10Z）で相手の ⑤ が ⑪ に入れ替わっている。

**これは #571 の手集計と本測定の差分の原因でもある。** #571 本文は同日 14:38 のスリープを理由に
朝 11:52 のスイープ（軸 ③・ROI 68.0%）でこのレースを評価していたが、ADR 0076 の測定は
発走前最終スイープ（13:15・軸 ⑥・ROI 31.8%）を採るため伝票そのものが別物になっていた。

### 実例 2: 軸は不変だが相手が動く（2026-08-09 新潟11R）

```
captured_at(UTC) | 軸 | 馬連の相手（軸を除く）
2026-08-09T08:13 |  3 | 1,2,4,9,16
2026-08-09T08:20 |  3 | 1,2,4,9,16
2026-08-09T08:28 |  3 | 1,2,4,9,16
2026-08-09T08:36 |  3 | 1,2,4,9,16
2026-08-09T08:43 |  3 | 1,2,9,10,16   ← ④ が抜け ⑩ が入る
```

**発走 5 分前**に相手の 5 頭目が ④ → ⑩ へ入れ替わっている。

## 原因

`compose_portfolio`（`src/use-case/src/interactor/race/predict/orchestrate.rs`）は
`build_portfolio(rank=blended, ev=pure, …)` を呼び、軸・相手は `rank_axis_partners` が
**`rank_probs`（市場ブレンド α=0.2）** から選ぶ。α はモデル重みなので市場が 0.8——
つまり選定は事実上市場人気に追随する。混戦判定 `konsen_band(rank_probs)` も同じ系列。

`src/apps/predict-watch/src/watch.rs` は `compose_portfolio(&views, &odds, race_budget, None)` と
**`forced_axis=None`** で呼んでいた（`recommend.rs` も同様）。軸を固定しているのは
`board.rs` だけで、#388（`4e84f87`）のコミットメッセージにも
「既存呼び出し（predict/watch/recommend）は default() 経由で不変」と明記がある。

なお `board.rs` の「買い目は既存経路（相手 top5 不変）」というコメントは**事実に反していた**
（相手は毎回 blended から選び直される）。本 issue で文言を実態に合わせた。

## 固定の材料が DB にあるか（設計判断の根拠）

| テーブル | 件数 | 最新 |
|---|---|---|
| `predictions`（人手予想 pad・◎の記録） | 35 | **2026-06-21** |
| `prediction_horses` | 487 | 〃 |
| `predict_bets` | **0** | — |
| `predict_sessions` | 10 | 2026-07-26 |
| `live_ev_snapshots` | 839 | 2026-08-09 |

**測定窓（2026-07-11〜08-09）に人手予想の記録は 1 件も無い。** よって
「記録◎を軸に固定する」（#388 が盤面でやっていること）だけでは predict-watch のフリップは
1 件も止まらない。固定は `live_ev_snapshots`（その日の初回スイープ）から自給する必要がある。

## 再現方法

```sh
# 軸・相手・混戦がスイープ間で動いたレース数
psql "$PADDOCK_DB_URL" -At -F'|' -c "
WITH legs AS (
  SELECT race_id, captured_at, axis, konsen,
         (SELECT string_agg(DISTINCT e::text, ',' ORDER BY e::text)
          FROM jsonb_array_elements(slip->'legs') leg, jsonb_array_elements(leg->'combo') e
          WHERE leg->>'bet_type'='quinella') AS partners
  FROM live_ev_snapshots),
a AS (SELECT race_id, count(*) n, count(DISTINCT axis) ax,
             count(DISTINCT partners) p, count(DISTINCT konsen) k
      FROM legs GROUP BY race_id)
SELECT '2スイープ以上', count(*)::text FROM a WHERE n>=2
UNION ALL SELECT '軸が動いた',   count(*)::text FROM a WHERE n>=2 AND ax>=2
UNION ALL SELECT '相手が動いた', count(*)::text FROM a WHERE n>=2 AND p>=2
UNION ALL SELECT '混戦が動いた', count(*)::text FROM a WHERE n>=2 AND k>=2;"
```

修正後の確認は `scripts/predict-check/gate_calibration.py`（#571 で追加）の
**「軸（◎）の安定性」節**が `0/N` になることで行う。
