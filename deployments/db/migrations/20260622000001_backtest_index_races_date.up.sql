-- via:no-schema-check: CREATE INDEX は非破壊的 DDL。既存インデックス一覧は
-- pg_indexes で確認済み（races に date/source インデックスが存在しない事を確認）。
-- idx_races_date: backtest の batch クエリ（results JOIN races WHERE races.date < $N）が
--   races に date インデックス無しで seq scan していたため追加（#195 計測で発覚）。
-- idx_races_source: fetch-card / fetch-results の既存確認クエリ（WHERE races.source = $N の等値検索）で
--   seq scan が発生しないよう追加。source カラムは UNIQUE 制約なし（同一ソースの再 fetch を許容する設計）。
-- CONCURRENTLY 不使用: sqlx はトランザクション内で migration を実行するため
--   CONCURRENTLY を使うとエラーになる（回避には -- no-transaction が必要でロールバック不可になる）。
CREATE INDEX IF NOT EXISTS idx_races_date ON races (date);
CREATE INDEX IF NOT EXISTS idx_races_source ON races (source);
