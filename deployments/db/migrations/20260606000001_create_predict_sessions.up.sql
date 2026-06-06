-- 予想セッション（1 開催日 = 1 セッション）。途中離脱後の --resume と --summary に使う。
-- balance/total_bet/total_payout はレース確定ごとに更新し、全レース処理で completed=1。
CREATE TABLE predict_sessions (
    date         TEXT PRIMARY KEY,           -- YYYY-MM-DD
    budget       INTEGER NOT NULL,           -- 開始予算（円）
    balance      INTEGER NOT NULL,           -- 現在残高（円）
    total_bet    INTEGER NOT NULL DEFAULT 0,
    total_payout INTEGER NOT NULL DEFAULT 0,
    completed    INTEGER NOT NULL DEFAULT 0, -- 0/1: 全レース処理済みか
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
