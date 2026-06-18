-- via:no-schema-check: スキーマ migration(DDL) の rollback 本体であり既存クエリではない。
-- #147 のライフサイクル状態を巻き戻す。downloaded 行は ingest 成功ではないため破棄する
-- （旧スキーマには「ダウンロード済み・未 ingest」という状態が存在し得ない）。
DELETE FROM fetch_history WHERE status <> 'ingested';

ALTER TABLE fetch_history
    DROP COLUMN status;
