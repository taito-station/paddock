ALTER TABLE results ADD COLUMN source   TEXT NOT NULL DEFAULT 'pdf';
ALTER TABLE results ADD COLUMN horse_id TEXT;
CREATE INDEX idx_results_horse_id ON results(horse_id);
