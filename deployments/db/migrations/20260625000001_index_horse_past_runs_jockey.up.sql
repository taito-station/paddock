-- via:no-schema-check: CREATE INDEX は非破壊的 DDL。horse_past_runs の既存インデックスは
-- idx_horse_past_runs_name(horse_name) / idx_horse_past_runs_date(date) のみで jockey 列が無い事を確認済み。
-- idx_horse_past_runs_jockey: 騎手直近フォーム（#221）の find_jockey_recent_runs / batch クエリが
--   horse_past_runs を `jockey = $N` / `jockey = ANY($N)` で絞る。jockey インデックス無しでは netkeiba
--   アームが seq scan になり predict/backtest の latency に効くため追加（results には idx_results_jockey が既存）。
-- CONCURRENTLY 不使用: sqlx はトランザクション内で migration を実行するため
--   CONCURRENTLY を使うとエラーになる。
CREATE INDEX IF NOT EXISTS idx_horse_past_runs_jockey ON horse_past_runs (jockey);
