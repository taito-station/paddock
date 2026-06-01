CREATE TABLE fetch_history (
    source_key   TEXT PRIMARY KEY,
    url          TEXT NOT NULL,
    races_saved  INTEGER NOT NULL DEFAULT 0,
    horses_saved INTEGER NOT NULL DEFAULT 0,
    fetched_at   TEXT NOT NULL
);
