-- セッション内で実際に購入した買い目。払戻は買い目ごと（per-bet）に記録し、
-- 命中精度・回収率の分析に使えるようにする。
CREATE TABLE predict_bets (
    bet_id       INTEGER PRIMARY KEY AUTOINCREMENT,
    session_date TEXT NOT NULL REFERENCES predict_sessions(date) ON DELETE CASCADE,
    race_id      TEXT NOT NULL,
    bet_type     TEXT NOT NULL,              -- win/place/quinella/exacta/trio/trifecta
    combination  TEXT NOT NULL,              -- "3" / "1-5" / "1>5" / "1-3-5" / "1>3>5"
    stake        INTEGER NOT NULL,           -- 賭け金（円）
    payout       INTEGER NOT NULL DEFAULT 0, -- 払戻（円, per-bet）
    ev           REAL NOT NULL,
    created_at   TEXT NOT NULL
);

CREATE INDEX idx_predict_bets_session_date ON predict_bets(session_date);
CREATE INDEX idx_predict_bets_race         ON predict_bets(session_date, race_id);
