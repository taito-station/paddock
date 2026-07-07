ALTER TABLE race_cards DROP CONSTRAINT IF EXISTS ck_race_cards_race_class;
ALTER TABLE race_cards DROP COLUMN IF EXISTS race_class;
