-- live オッズの時系列スナップショットを履歴保持する append-only テーブル（#232）。
-- race_odds は PK=(race_id,bet_type,combination_key) の単一行 UPSERT で最新値しか残らず、
-- 締切前 live を取っても後続/事後フェッチ（確定オッズ）に上書きされ消える。fetched_at を PK に
-- 含めることで別時刻の取得を別行として積み、live が事後フェッチで消えない構造にする。
-- #218（live オッズで α 再校正）の入力データを蓄積するための基盤。
-- カラム構成は race_odds と同一。fetched_at は UTC の rfc3339 文字列で保存され
-- （save_race_odds が DateTime<Utc>::to_rfc3339() で書く＝常に +00:00・固定書式）、
-- 辞書順=時刻順になることを前提に PK・index・ORDER BY を組む。既存 race_odds /
-- find_race_odds の as_of 比較（substr(fetched_at,1,10)）と同一の TEXT 時刻規約。
CREATE TABLE IF NOT EXISTS race_odds_snapshots (
    race_id         TEXT NOT NULL,
    bet_type        TEXT NOT NULL,
    combination_key TEXT NOT NULL,
    odds            DOUBLE PRECISION NOT NULL,
    odds_high       DOUBLE PRECISION,
    popularity      BIGINT,
    fetched_at      TEXT NOT NULL,
    PRIMARY KEY (race_id, bet_type, combination_key, fetched_at)
);
-- race_id で 1 レース分を時系列順（fetched_at 昇順）に引くための index。PK 先頭は
-- race_id だが fetched_at は 4 列目のため、PK 前方一致では時系列取得を賄えず別途必要。
CREATE INDEX IF NOT EXISTS idx_race_odds_snapshots_race ON race_odds_snapshots(race_id, fetched_at);
