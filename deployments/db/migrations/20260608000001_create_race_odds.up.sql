CREATE TABLE race_odds (
    race_id         TEXT    NOT NULL,
    bet_type        TEXT    NOT NULL,
    combination_key TEXT    NOT NULL,
    odds            REAL    NOT NULL,
    odds_high       REAL,
    popularity      INTEGER,
    fetched_at      TEXT    NOT NULL,
    PRIMARY KEY (race_id, bet_type, combination_key)
);
