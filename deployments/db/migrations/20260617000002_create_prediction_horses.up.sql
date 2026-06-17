-- 予想の馬ごと行（印・短評・確率・単勝/人気）。#145 の馬名/印 検索のため horse_name/mark に索引。
-- 確率・単勝・人気は予想時に分からないことがあるため nullable。
CREATE TABLE prediction_horses (
    prediction_id INTEGER NOT NULL REFERENCES predictions(prediction_id) ON DELETE CASCADE,
    horse_num     INTEGER NOT NULL,
    horse_name    TEXT NOT NULL,
    jockey        TEXT,
    mark          TEXT,                       -- honmei/taikou/tanana/renge/hoshi/chui
    win_odds      REAL,                       -- 単勝オッズ
    popularity    INTEGER,                    -- 人気
    win_prob      REAL,                       -- 勝率
    place_prob    REAL,                       -- 連対率
    show_prob     REAL,                       -- 複勝率
    comment       TEXT,                       -- 短評
    PRIMARY KEY (prediction_id, horse_num)
);

CREATE INDEX idx_prediction_horses_name ON prediction_horses(horse_name);
CREATE INDEX idx_prediction_horses_mark ON prediction_horses(mark);
