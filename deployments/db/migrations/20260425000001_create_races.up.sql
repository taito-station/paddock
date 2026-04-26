CREATE TABLE races (
    race_id          TEXT PRIMARY KEY,
    date             TEXT NOT NULL,
    venue            TEXT NOT NULL,
    round            INTEGER NOT NULL,
    day              INTEGER NOT NULL,
    race_num         INTEGER NOT NULL,
    surface          TEXT NOT NULL,
    distance         INTEGER NOT NULL,
    track_condition  TEXT,
    weather          TEXT
);
CREATE INDEX idx_races_course ON races(venue, distance, surface);
CREATE INDEX idx_races_condition ON races(track_condition);
