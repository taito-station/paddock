-- live オッズの時系列スナップショットを履歴保持する append-only テーブル（#232）。
-- race_odds は PK=(race_id,bet_type,combination_key) の単一行 UPSERT で最新値しか残らず、
-- 締切前 live を取っても後続/事後フェッチ（確定オッズ）に上書きされ消える。fetched_at を PK に
-- 含めることで別時刻の取得を別行として積み、live が事後フェッチで消えない構造にする。
-- #218（live オッズで α 再校正）の入力データを蓄積するための基盤。
-- カラム構成は race_odds と同一。
CREATE TABLE race_odds_snapshots (
    race_id         TEXT NOT NULL,
    bet_type        TEXT NOT NULL,
    combination_key TEXT NOT NULL,
    odds            DOUBLE PRECISION NOT NULL,
    odds_high       DOUBLE PRECISION,
    popularity      BIGINT,
    fetched_at      TEXT NOT NULL,
    PRIMARY KEY (race_id, bet_type, combination_key, fetched_at)
);
CREATE INDEX idx_race_odds_snapshots_race ON race_odds_snapshots(race_id, fetched_at);
