-- via:no-schema-check: スキーマ migration(DDL) 本体であり既存クエリではない。現行 fetch_history 定義は
-- baseline.up.sql + 20260618000002_fetch_history_lifecycle.up.sql が一次情報。
-- #170 / ADR0024 論点1: 取得失敗(403/404)を「再試行の入力」として記録できるようにする。
--   status に 'failed' を追加（再試行対象であって除外フラグではない）。
--   http_status: failed 行の 403/404（downloaded/ingested 等の成功行は NULL）。
--   attempts: 失敗試行の累積回数（再試行/バックオフ判断の入力）。
--   last_attempt_at: 直近の試行時刻（時刻比較用）。
-- fetched_at は時刻比較のため TEXT(RFC3339) → TIMESTAMPTZ 化し、成功時のみ入る値として NOT NULL を外す
-- （純 failed 行は成功時刻を持たない）。既存行はすべて RFC3339 文字列なので USING ::timestamptz で変換可。

ALTER TABLE fetch_history
    DROP CONSTRAINT fetch_history_status_check;
ALTER TABLE fetch_history
    ADD CONSTRAINT fetch_history_status_check
        CHECK (status IN ('downloaded', 'ingested', 'failed'));

ALTER TABLE fetch_history
    ALTER COLUMN fetched_at TYPE TIMESTAMPTZ USING fetched_at::timestamptz,
    ALTER COLUMN fetched_at DROP NOT NULL;

ALTER TABLE fetch_history
    ADD COLUMN http_status     INTEGER,
    ADD COLUMN attempts        INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN last_attempt_at TIMESTAMPTZ;
