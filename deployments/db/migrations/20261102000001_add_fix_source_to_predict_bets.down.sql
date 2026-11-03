ALTER TABLE predict_bets
    DROP CONSTRAINT IF EXISTS ck_predict_bets_fix_odds_fetched_at,
    DROP CONSTRAINT IF EXISTS ck_predict_bets_fix_source,
    DROP COLUMN IF EXISTS fix_odds_fetched_at,
    DROP COLUMN IF EXISTS fix_source;
