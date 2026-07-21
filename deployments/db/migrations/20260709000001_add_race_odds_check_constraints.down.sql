ALTER TABLE race_odds DROP CONSTRAINT IF EXISTS ck_race_odds_odds_range;
ALTER TABLE race_odds DROP CONSTRAINT IF EXISTS ck_race_odds_odds_high_range;

ALTER TABLE race_odds_snapshots DROP CONSTRAINT IF EXISTS ck_race_odds_snapshots_odds_range;
ALTER TABLE race_odds_snapshots DROP CONSTRAINT IF EXISTS ck_race_odds_snapshots_odds_high_range;
