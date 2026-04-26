CREATE TABLE results (
    result_id           INTEGER PRIMARY KEY AUTOINCREMENT,
    race_id             TEXT NOT NULL REFERENCES races(race_id) ON DELETE CASCADE,
    finishing_position  INTEGER,
    gate_num            INTEGER NOT NULL,
    horse_num           INTEGER NOT NULL,
    horse_name          TEXT NOT NULL,
    jockey              TEXT,
    trainer             TEXT,
    time_seconds        REAL,
    margin              TEXT,
    odds                REAL,
    horse_weight        INTEGER,
    weight_change       INTEGER,
    UNIQUE(race_id, horse_num)
);
CREATE INDEX idx_results_horse  ON results(horse_name);
CREATE INDEX idx_results_jockey ON results(jockey);
CREATE INDEX idx_results_gate   ON results(gate_num);
