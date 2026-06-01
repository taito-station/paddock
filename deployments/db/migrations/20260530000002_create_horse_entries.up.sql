CREATE TABLE horse_entries (
    entry_id   INTEGER PRIMARY KEY AUTOINCREMENT,
    race_id    TEXT NOT NULL REFERENCES race_cards(race_id) ON DELETE CASCADE,
    gate_num   INTEGER NOT NULL,
    horse_num  INTEGER NOT NULL,
    horse_name TEXT NOT NULL,
    jockey     TEXT,
    UNIQUE(race_id, horse_num)
);

CREATE INDEX idx_horse_entries_race_id    ON horse_entries(race_id);
CREATE INDEX idx_horse_entries_horse_name ON horse_entries(horse_name);
