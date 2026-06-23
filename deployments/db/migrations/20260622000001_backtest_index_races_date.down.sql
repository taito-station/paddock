-- via:no-schema-check: DROP INDEX は非破壊的ロールバック。
DROP INDEX IF EXISTS idx_races_date;
DROP INDEX IF EXISTS idx_races_source;
