-- 朝の選定（軸・相手・混戦）を、レースごとに 1 行だけ記録する（#767）。
--
-- predict-watch は、監視窓に入る前に保存された「朝」の単勝スナップショット
-- （当日 JST 0 時以降で最初の単勝・#724 と同じ定義）から買い目を組み、その選定をここに保存する。
-- 以降のスイープと盤（/races/{id}/board）はこの行を固定の元にする（REQ-D01-003・REQ-D23-007）。
--
-- live_ev_snapshots に入れない理由: あちらは「レースの最古行＝監視窓に入った初回スイープ」を前提に
-- 計測（gate_calibration・#724 の再構成）と盤のフリップ判定が読んでいるため。
-- #448 の盤の「朝↔現」（最初に全券種がそろった時点）とは別の「朝」なので、名前も分けている。
--
-- 行は先勝ちで、一度書いたら上書きしない（保存は ON CONFLICT DO NOTHING）。再起動や --once の後も
-- 同じ選定を使うため。時刻は既存テーブルと同じ TEXT 規約（UTC rfc3339）に揃える:
--   odds_fetched_at … 朝のスナップショットの race_odds_snapshots.fetched_at をそのまま写す
--   computed_at     … 選定を計算した時刻
CREATE TABLE morning_selections (
    race_id         TEXT PRIMARY KEY,
    date            TEXT NOT NULL,      -- 'YYYY-MM-DD'（races.date と同じ規約）
    axis            BIGINT NOT NULL,    -- 軸の馬番
    partners        BIGINT[] NOT NULL,  -- 相手の馬番（昇順・軸を含まない）
    konsen_band     BIGINT[] NOT NULL,  -- 混戦のボックスの馬番（昇順・軸を含む）。空なら混戦ではない
    odds_fetched_at TEXT NOT NULL,
    computed_at     TEXT NOT NULL
);

CREATE INDEX idx_morning_selections_date ON morning_selections (date);
