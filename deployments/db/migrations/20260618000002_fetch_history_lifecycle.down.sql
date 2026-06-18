-- via:no-schema-check: スキーマ migration(DDL) の rollback 本体であり既存クエリではない。
-- #147 のライフサイクル状態を巻き戻す。downloaded 行は ingest 成功ではないため破棄する
-- （旧スキーマには「ダウンロード済み・未 ingest」という状態が存在し得ない）。
-- 運用注記: rollback 時に inbox に残る未 ingest PDF は DB 記録だけ消えて孤児化する。
-- 再 up 後の二重処理を避けるため、down を実行するなら inbox を空にしてから行うこと。
DELETE FROM fetch_history WHERE status <> 'ingested';

ALTER TABLE fetch_history
    DROP COLUMN status;
