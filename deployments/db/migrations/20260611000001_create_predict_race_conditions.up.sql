-- 予想セッションで各レース冒頭に対話入力した馬場状態（#80）。
-- races.track_condition は未確定レースでは構造的に NULL のため、「どの馬場前提で
-- 確率・買い目を出したか」を再現・監査するにはセッション入力を別途残す必要がある。
-- 行の存在 = そのレースで入力済み。track_condition IS NULL = 「不明」として記録した状態
--（未入力＝行なし、とは区別する）。
CREATE TABLE predict_race_conditions (
    session_date    TEXT NOT NULL REFERENCES predict_sessions(date) ON DELETE CASCADE,
    race_id         TEXT NOT NULL,
    track_condition TEXT,                       -- 良/稍重/重/不良。NULL=不明として記録
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    PRIMARY KEY (session_date, race_id)
);
