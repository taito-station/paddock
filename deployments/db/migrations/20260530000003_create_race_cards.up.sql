CREATE TABLE race_cards (
    race_id   TEXT PRIMARY KEY,
    venue     TEXT NOT NULL,
    round     INTEGER NOT NULL,
    day       INTEGER NOT NULL,
    race_num  INTEGER NOT NULL,
    surface   TEXT NOT NULL,
    distance  INTEGER NOT NULL
);
