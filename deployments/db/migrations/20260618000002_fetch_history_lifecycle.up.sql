-- via:no-schema-check: スキーマ migration(DDL) 本体であり既存クエリではない。現行 fetch_history 定義は baseline.up.sql(本セッションで確認済み) が一次情報。
-- #147: fetch/parse ステージ分割のため fetch_history に取得ライフサイクルの状態を持たせる。
-- 旧 fetch_history は「ingest 成功した開催日」だけを記録する成功ログだった。Stage1(ダウンロード
-- のみ)で inbox に置いた未 ingest の開催日も同じテーブルで表現し、dedup を 1 テーブルに統一する。
--   downloaded: PDF を inbox に保存済みだが未 ingest（Stage1 完了）
--   ingested  : parse+保存まで完了（Stage2 完了）
-- 失敗(403/404 等)の追跡は別 Issue（ADR0024 論点1）。本マイグレーションでは扱わない。

ALTER TABLE fetch_history
    ADD COLUMN status TEXT NOT NULL DEFAULT 'ingested'
        CHECK (status IN ('downloaded', 'ingested'));
-- 既存行はすべて ingest 成功ログなので DEFAULT 'ingested' で正しい。
-- CHECK で不正値を DB レベルで弾く（不明値が dedup を黙って無効化するのを防ぐ）。
