-- 予想（印・短評・買い目・結果）の構造化レコード。DB を正とし、pad の MD はここから生成する。
-- レース同定は (date, venue, race_num) で一意（pad パス由来）。race_id は races/race_cards に
-- 一致が見つかった時のみ解決して保持（未確定レースや未取込レースでは NULL）。
CREATE TABLE predictions (
    prediction_id INTEGER PRIMARY KEY AUTOINCREMENT,
    date          TEXT NOT NULL,              -- YYYY-MM-DD
    venue         TEXT NOT NULL,              -- 日本語の場名（races.venue と同形式）
    race_num      INTEGER NOT NULL,
    race_id       TEXT,                       -- races 照合で解決できた時のみ
    title         TEXT,                       -- H1 のレース名/クラス
    budget        INTEGER,                    -- 予算（円）
    strategy_note TEXT,                       -- 買い目の狙い/方針
    commentary    TEXT,                       -- 敗因分析等の自由記述（生成 MD 末尾に出す）
    finish_1      INTEGER,                    -- 結果 1 着（馬番）
    finish_2      INTEGER,                    -- 結果 2 着（馬番）
    finish_3      INTEGER,                    -- 結果 3 着（馬番）
    recovery_rate REAL,                       -- 回収率（%）
    pnl           INTEGER,                    -- 収支（円, 符号付き）
    result_note   TEXT,                       -- 結果コメント（自由記述）
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    UNIQUE(date, venue, race_num)
);
