-- netkeiba 近走 1 走 = 1 行。pdf 確定成績(results)から物理分離し、混在による
-- 二重計上・フィールドバイアスを構造的に解消する。列は HorsePastRun に対応。
CREATE TABLE horse_past_runs (
    horse_id            TEXT NOT NULL REFERENCES horses(horse_id) ON DELETE CASCADE,
    race_id             TEXT NOT NULL,
    netkeiba_race_id    TEXT NOT NULL,
    date                TEXT NOT NULL,
    venue               TEXT NOT NULL,
    round               INTEGER NOT NULL,
    day                 INTEGER NOT NULL,
    race_num            INTEGER NOT NULL,
    surface             TEXT NOT NULL,
    distance            INTEGER NOT NULL,
    track_condition     TEXT,
    finishing_position  INTEGER,
    status              TEXT NOT NULL,
    gate_num            INTEGER NOT NULL,
    horse_num           INTEGER NOT NULL,
    horse_name          TEXT NOT NULL,
    jockey              TEXT,
    time_seconds        REAL,
    margin              TEXT,
    odds                REAL,
    horse_weight        INTEGER,
    weight_change       INTEGER,
    weight_carried      REAL,
    popularity          INTEGER,
    PRIMARY KEY (horse_id, race_id)
);
CREATE INDEX idx_horse_past_runs_name ON horse_past_runs(horse_name);
CREATE INDEX idx_horse_past_runs_date ON horse_past_runs(date);
