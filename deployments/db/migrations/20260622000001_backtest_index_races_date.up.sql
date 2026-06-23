-- via:no-schema-check: CREATE INDEX は非破壊的 DDL。既存インデックス一覧は
-- pg_indexes で確認済み（races に date/source インデックスが存在しない事を確認）。
-- idx_races_date: backtest の batch クエリ（results JOIN races WHERE races.date < $N）が
--   races に date インデックス無しで seq scan していたため追加（#195 計測で発覚）。
-- idx_races_source: fetch-card / fetch-results の重複チェック（WHERE races.source = $N）で
--   seq scan が発生しないよう追加。
CREATE INDEX idx_races_date ON races (date);
CREATE INDEX idx_races_source ON races (source);
