-- via:no-schema-check: CREATE INDEX は非破壊的 DDL。
-- idx_results_trainer_pattern: save_race_card の調教師名正規化(step2)が
--   `trainer LIKE $1 || '%'` で前方一致検索する。DB照合順序が en_US.utf8 のため
--   既存 idx_results_trainer(btree) では前方一致 LIKE に効かず Seq Scan(約68k行)になる。
--   text_pattern_ops btree で前方一致専用比較(~>=~/~<~)を index 化する(#289)。
-- 部分 index の WHERE はクエリの `AND trainer IS NOT NULL` と既存 idx に合わせる。
-- CONCURRENTLY 不使用: sqlx はトランザクション内で migration を実行するため。
CREATE INDEX IF NOT EXISTS idx_results_trainer_pattern
    ON results (trainer text_pattern_ops)
    WHERE trainer IS NOT NULL;
