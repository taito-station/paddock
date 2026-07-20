ALTER TABLE race_odds DROP CONSTRAINT IF EXISTS ck_race_odds_odds;
ALTER TABLE race_odds DROP CONSTRAINT IF EXISTS ck_race_odds_odds_high;
ALTER TABLE race_odds DROP CONSTRAINT IF EXISTS ck_race_odds_band;

ALTER TABLE race_odds_snapshots DROP CONSTRAINT IF EXISTS ck_race_odds_snapshots_odds;
ALTER TABLE race_odds_snapshots DROP CONSTRAINT IF EXISTS ck_race_odds_snapshots_odds_high;
ALTER TABLE race_odds_snapshots DROP CONSTRAINT IF EXISTS ck_race_odds_snapshots_band;
