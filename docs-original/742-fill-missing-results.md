# #742 results の行が足りない 16 レースの補完 — 実測ログ（2026-10-03）

足りない馬番の行だけを netkeiba の結果ページから INSERT し、既存行には触れなかった記録。
方針の確認は `qa/QA-fill-results-742.md`、蒸留先は `knowledge/race-result-ingestion.md`（決定ログ #742）と
`knowledge/netkeiba-datasource.md`（結果ページの生の HTML の取得）。

## 1. 対象の確認（読み取り専用・共有 DB）

条件「出馬表（`horse_entries`）の頭数 > `results` の行数」で絞ると、共有 DB では **16 本だけ**が当たった（issue の 16 本と一致）。

```sql
SET default_transaction_read_only = on;
WITH e AS (SELECT race_id, count(*) n FROM horse_entries GROUP BY race_id),
     r AS (SELECT race_id, count(*) n FROM results GROUP BY race_id)
SELECT r.race_id FROM r JOIN e USING (race_id) JOIN race_cards rc USING (race_id)
WHERE e.n > r.n AND r.n <= 5 ORDER BY rc.date, r.race_id;
```

- 16 本とも races 行があり、既存の行はすべて `status='finished'`（1 行が 12 本・5 行が 4 本）。
- `r.n <= 5` を外しても同じ 16 本だった（行数が出馬表より少ないレースは他に無い）。足りない行は合計 203 行（1 レース 6〜17 行）。
- #730 の「行数が馬番の最大値より少ないレース（dev 81R・test 91R）」は、#730 が dev・test の窓について数えた別の指標（馬番の最大値との比較）。
  この条件（出馬表の頭数との比較）では共有 DB に当たらなかった。今回は触れていない。
- 対象一覧（16 本・ヘッダ付き）を凍結した: `~/paddock-backups/pinned/742-fill-results/targets.txt`
  （sha256 `b9fc1c9fa9582cf42556088ded88830d788f3ce728f9604878cea67479d1d506`）。

## 2. 取得（netkeiba の結果ページ）

- コマンド: `cargo run -q -p fill-results -- fetch --targets <targets.txt> --out-dir ~/paddock-backups/pinned/742-fill-results --interval-ms 3500`
  （ブランチ `fix/742-fill-missing-results` の debug ビルド）。
- 2026-10-03 20:10 JST に 1 回だけ実行。16 リクエスト・間隔 3,500ms・再送なし・並列なし。16 本とも保存し、取得失敗・読めないページは 0。
- 保存先: `~/paddock-backups/pinned/742-fill-results/raw/<race_id>.html`。sha256 の一覧は同じ場所の `sha256.txt`（16 行）。

## 3. 隔離 DB での検証

共有 DB を `scripts/backup-db.sh` で dump し（`~/paddock-backups/paddock-20261003-201101.dump`）、同じコンテナの PG17 に隔離 DB を作って復元した。

```sh
DUMP=~/paddock-backups/paddock-20261003-201101.dump
limactl shell paddock -- nerdctl exec paddock-postgres createdb -U paddock paddock_test742
limactl shell paddock -- nerdctl exec -i paddock-postgres pg_restore -U paddock -d paddock_test742 < "$DUMP"
```

- 復元直後: results 76,392 行・races 5,509 行（共有 DB と一致）。適用前の `max(result_id)` は 86932。
- 照合の SQL（適用前後で同じものを流す）:
  - 既存行: `result_id <= 86932` の行の `row_to_json` を `result_id` 順に連結した md5（全列）。
  - races: 全行の `row_to_json` を `race_id` 順に連結した md5（全列）。
  - 新しい行: `result_id > 86932` の行数・レース数・`source <> 'pdf'` または `horse_id`・`margin` が NULL でない行の数、16 本以外に入った行の数。
  - レースごとの出走馬（`finished`・`did_not_finish`）の行数・全行数・出馬表の頭数。
- 適用: `PADDOCK_DB_URL=<…>/paddock_test742 cargo run -q -p fill-results -- apply --targets … --out-dir … --db-name paddock_test742`（先に `--dry-run`）。

| 確かめたこと | 結果 |
|---|---|
| dry-run | 16 本とも照合を通過・INSERT 予定 203 行（失敗 0） |
| 既存行の md5（全列） | 前後とも `e13120d55e3de0c2a25b6a308e9d7c17`（不変） |
| races の md5（全列） | 前後とも `a84571d1939115fac4d91db8e62f6695`（不変・5,509 行） |
| 新しい行 | 203 行・16 レース・`source='pdf'`／`horse_id`・`margin` NULL 以外は 0・16 本以外は 0 |
| 出走馬の行数 | 16 本とも結果ページの出走馬の数と一致（下表） |
| 2 回目の apply | 0 行（冪等） |
| `--db-name paddock` で隔離 DB に接続 | 書かずに終了コード 1 |

新しい行の status: `finished` 200・`did_not_finish` 1（福島 8 日 7R の 14 番）・`scratched` 2（中京 6 日 11R の 12 番・12R の 7 番）。
取消・除外の馬も、出馬表と結果ページの両方にあるので status ごと入れた（質問票 Q2）。

| race_id | 既存 | INSERT | 出走馬の行数（後） | 全行数（後） | 出馬表 |
|---|---|---|---|---|---|
| 2026-1-hakodate-11-12R | 1 | 11 | 12 | 12 | 12 |
| 2026-2-fukushima-8-7R | 5 | 10 | 15 | 15 | 15 |
| 2026-2-niigata-2-4R | 5 | 13 | 18 | 18 | 18 |
| 2026-1-sapporo-4-12R | 1 | 13 | 14 | 14 | 14 |
| 2026-1-sapporo-6-10R | 1 | 13 | 14 | 14 | 14 |
| 2026-1-sapporo-6-1R | 1 | 11 | 12 | 12 | 12 |
| 2026-2-chukyo-6-10R | 1 | 15 | 16 | 16 | 16 |
| 2026-2-chukyo-6-11R | 5 | 13 | 17 | 18 | 18 |
| 2026-2-chukyo-6-12R | 1 | 15 | 15 | 16 | 16 |
| 2026-2-chukyo-6-1R | 1 | 15 | 16 | 16 | 16 |
| 2026-2-chukyo-6-6R | 1 | 6 | 7 | 7 | 7 |
| 2026-2-niigata-6-11R | 1 | 17 | 18 | 18 | 18 |
| 2026-2-niigata-6-12R | 1 | 17 | 18 | 18 | 18 |
| 2026-2-niigata-6-1R | 5 | 11 | 16 | 16 | 16 |
| 2026-2-niigata-6-6R | 1 | 12 | 13 | 13 | 13 |
| 2026-2-niigata-6-7R | 1 | 11 | 12 | 12 | 12 |

照合の後、隔離 DB は削除した（`dropdb`。§6 の接続の残りを切ってから）。

## 4. 共有 DB への適用（ユーザー承認後）

- 直前のバックアップ: `scripts/backup-db.sh` → `~/paddock-backups/paddock-20261003-201949.dump` を
  `~/paddock-backups/pinned/pre742-results-20261003-201949.dump`（行数サイドカー付き）に複製した（日次の世代管理の対象外）。
- 適用前の `max(result_id)` は隔離 DB と同じ 86932。dry-run の計画は隔離 DB の結果とレースごとに一致した。
- `apply --db-name paddock` の出力は隔離 DB のときと完全に一致（合計 203 行・失敗 0）。
- 適用後の照合結果（既存行と races の md5・新しい行・レースごとの行数）も、隔離 DB の適用後と完全に一致した。

## 5. 戻し方

戻すのは `results` 表だけにする（全体復元は、以後に貯めた `race_odds_snapshots` などまで巻き戻すので使わない。#730 と同じ）。

```sh
D=~/paddock-backups/pinned/pre742-results-20261003-201949.dump
limactl shell paddock -- nerdctl exec paddock-postgres psql -U paddock -d paddock -c "TRUNCATE results"
limactl shell paddock -- nerdctl exec -i paddock-postgres pg_restore -U paddock -d paddock --data-only -t results < "$D"
```

今回の補完だけを取り消すなら、新しい行だけを消せば足りる（既存行には触れていないため）:
`DELETE FROM results WHERE result_id > 86932 AND race_id IN (<16 本>)`。ただし、以後に別の経路で 16 本に入った行も消えるので、実行前に件数を確かめる（203 のはず）。

## 6. 残る事項・観測

- 16 本の races 行の `track_condition`・`weather` は埋めていない（#745 の「馬場不明」として扱われる）。スコープ外。
- 評価・backtest の母集合と本番の予想の入力が変わった。16 本を除外して測った既存の値は、§4 の dump を使うか
  `result_id > 86932` の行を除いて数えないと再現できない（決定ログ #742 の「影響」）。
- 実行のたびに DB 接続が残った（#717）:
  - `#[sqlx::test]` が panic で失敗すると、一時 DB に接続が 2 本ずつ残り、次の実行で一時 DB を作り直せなかった。
  - `apply` を 4 回走らせた後、隔離 DB に idle の接続が 10 本残っていた。
  - その時点で `paddock` DB に idle が 78 本（合計 89/100）あった。ユーザー承認のうえ #717 の手順で掃除した。

## 7. 取得と適用に使ったバイナリについて（後から判明・2026-10-03）

§2〜§4 の fetch・apply は、**変異テストの変異が残ったビルド**で実行していた。変異テストのスクリプトがファイルを `cp`→`mv` で戻したため、
戻したファイルの更新時刻が変異前のままになり、cargo が再ビルドしなかった。残っていた変異は各 crate の最後の 1 本:

| crate | 残っていた変異 | §2〜§4 への影響 |
|---|---|---|
| netkeiba-scraper | M1: 結果ページの取得が共有のリトライを通る（再送する） | 再送すると scraper-util が `warn` を出すが、fetch の出力（全 19 行）に `warn` は 0。16 リクエストのまま・再送なし |
| rdb-gateway | M3: `insert_missing_results` の races 行の確認が無効 | 16 本とも races 行があることを §1 で確認済み。確認が効いても結果は同じ |
| fill-results | M13: 結果の表を読めないページも保存する | apply で 16 本ともパースできた（読めないページは保存されていない） |

- 全テストで自作の結合テスト（races 行が無ければエラー）が FK 違反で落ちて露見した。ファイルを `touch` して正規のビルドに戻し、
  対象のテスト（rdb-gateway の結合テスト 3・fill-results 16・netkeiba-scraper 27）が通ることを確かめた。
- 正規のビルドで共有 DB に `apply --dry-run` を流し、16 本とも照合を通過・INSERT 予定 0 行（補完済みの状態と整合）を確かめた。
- 再送の `warn` の観測元: ログは `paddock-config` の `init_tracing`（tracing_subscriber の既定の出力先＝標準出力）に出る。
  §2 の fetch は標準出力と標準エラーをまとめて取っており、19 行は見出し 1・進捗 16・集計 1・完了 1 の `println!` とちょうど一致する。
- 変異の番号は M1〜M13 で、M4（出馬表に無い馬番も入れる）は、書いた書き換え式が型の上で成り立たないと判断して実行前に外した（その分岐は結合テスト「出馬表に無い 4 番は入れない」が直接確かめている）。実行した 12 本はすべて検出された。
- 残っていた 3 本はいずれも、INSERT する列と値・照合（`plan_race`）には触れていない。
- 以上から、共有 DB に入った 203 行は正規のビルドで入れた場合と同じ。データの修正は不要と判断した。
- 再発防止: 変異を戻した後は `touch` して更新時刻を進め（または git で戻し）、本番データに触る前に正規のビルドで対象のテストを流し直す
  （作業者の手順。リポジトリには変異テストの仕組みを置いていないので、手順の記録はここと作業メモに残した）。

## 8. レビュー反映後のビルドでの再確認（2026-10-03）

- §7 の「正規のビルドでの dry-run」は、レビュー 1 巡目の反映（races 行の確認 `FillStore::race_exists` の追加）より前のビルドで取ったもの。
  1 巡目の反映後のビルドは、既存の `RaceRepository::race_exists` が `SELECT 1`（INT4）を i64 で受けていたため、races 行があるレースの
  1 本目で型の不一致により止まった（共有 DB への dry-run で再現。書き込みは無い）。2 巡目で i32 に直し、実 DB で `apply` を通す結合テストを足した。
- 2 巡目の反映後のビルド（コミット 6b083b4）で共有 DB に `apply --dry-run` を流し、16 本とも照合を通過・INSERT 予定 0 行・失敗 0 を確かめた。
  このビルドの照合には「既存行の着順・status が結果ページの同じ馬番の行と一致する」が加わっているので、16 本の全行（補完前からの行と
  補った 203 行）の着順と status が保存した HTML と一致していることも確かめたことになる。
- 16 本の中で同じ着順が 2 行あるのは 2 か所（函館 11 日 12R の 2 着・新潟 6 日 12R の 5 着）。どちらも結果ページ自体の同着
  （次の着順が 1 つ飛ぶ: 2・2・4 と 5・5・7）で、補った行どうしの重複。

