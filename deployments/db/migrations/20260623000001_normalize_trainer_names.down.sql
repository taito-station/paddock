-- 略名への逆変換は不可逆のため no-op とする。
-- horse_entries.trainer を元の略名に戻す手段がない（略名はデータとして保持していない）。
-- ロールバックが必要な場合は、コードを #219 以前にリバートしてから fetch-card を再実行すること
-- （再実行後は netkeiba の略名が horse_entries.trainer に入る状態に戻る）。
SELECT 1;
