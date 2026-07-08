ALTER TABLE live_ev_snapshots DROP CONSTRAINT IF EXISTS ck_live_ev_snapshots_roughness;
ALTER TABLE live_ev_snapshots DROP COLUMN IF EXISTS roughness;
