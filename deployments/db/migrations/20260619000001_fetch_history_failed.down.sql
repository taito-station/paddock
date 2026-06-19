-- via:no-schema-check: スキーマ migration(DDL) の rollback 本体であり既存クエリではない。
-- #170 の failed 追跡を巻き戻す。failed 行は旧スキーマに存在し得ない状態（かつ fetched_at が NULL）
-- なので破棄する。残る downloaded/ingested 行は必ず fetched_at を持つため SET NOT NULL に通る。
DELETE FROM fetch_history WHERE status = 'failed';

ALTER TABLE fetch_history
    DROP COLUMN http_status,
    DROP COLUMN attempts,
    DROP COLUMN last_attempt_at;

ALTER TABLE fetch_history
    DROP CONSTRAINT fetch_history_status_check;
ALTER TABLE fetch_history
    ADD CONSTRAINT fetch_history_status_check
        CHECK (status IN ('downloaded', 'ingested'));

-- ::text は timestamptz を ' ' 区切り（例 '2026-06-19 00:00:00+00'）で出力し、元の
-- to_rfc3339()（'T' 区切り）と書式が完全一致しない＝厳密には非可逆。ただし
-- fetch_history.fetched_at を読み戻して RFC3339 でパースする消費者はコード上存在せず
-- （write-only、dedup は status 列で判定）、ロールバック後も実害は無い。
ALTER TABLE fetch_history
    ALTER COLUMN fetched_at TYPE TEXT USING fetched_at::text,
    ALTER COLUMN fetched_at SET NOT NULL;
