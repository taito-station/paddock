-- 注: ALTER TABLE ... DROP COLUMN は SQLite 3.35.0 (2021) 以降が必要。
-- 本プロジェクトは sqlx 同梱の新しい libsqlite3 を使うため要件を満たす。
-- up でバックフィルした date 値は列削除に伴い失われる（rollback 用途のため許容）。
ALTER TABLE race_cards DROP COLUMN date;
