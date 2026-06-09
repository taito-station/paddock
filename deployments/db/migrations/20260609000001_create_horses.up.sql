CREATE TABLE horses (
    horse_id    TEXT PRIMARY KEY,
    horse_name  TEXT NOT NULL
);
CREATE INDEX idx_horses_name ON horses(horse_name);
