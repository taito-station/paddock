-- 予想の買い目行。組合せは arabic 馬番のハイフン連結（"7" / "7-14" / "7-14-13"）で保持する。
-- ordinal で MD 上の並び順を保つ。
CREATE TABLE prediction_bets (
    prediction_id INTEGER NOT NULL REFERENCES predictions(prediction_id) ON DELETE CASCADE,
    ordinal       INTEGER NOT NULL,
    bet_type      TEXT NOT NULL,              -- 単勝/複勝/馬連/ワイド/馬単/3連複/3連単
    combination   TEXT NOT NULL,              -- "7" / "7-14" / "7-14-13"
    amount        INTEGER NOT NULL,           -- 金額（円）
    PRIMARY KEY (prediction_id, ordinal)
);
