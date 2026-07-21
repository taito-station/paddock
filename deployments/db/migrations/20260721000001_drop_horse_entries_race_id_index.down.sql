-- via:no-schema-check: CREATE INDEX は非破壊的ロールバック。
-- baseline 20260618000001 line 80 の元定義をそのまま復元する（#471 のロールバック）。
CREATE INDEX idx_horse_entries_race_id ON horse_entries (race_id);
