ALTER TABLE race_odds DROP CONSTRAINT IF EXISTS ck_race_odds_bet_type;

ALTER TABLE race_odds_snapshots DROP CONSTRAINT IF EXISTS ck_race_odds_snapshots_bet_type;

ALTER TABLE live_ev_snapshots DROP CONSTRAINT IF EXISTS ck_live_ev_snapshots_verdict;
