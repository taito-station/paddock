-- via:no-schema-check: local SQLite migration. horse_past_runs/races/results のスキーマは
-- 本セッションで確認済み（create_* + add_* migration）。
--
-- horse_past_runs から races/results へ source='netkeiba' として戻す。
-- （この down は create_horse_past_runs.down より先に走るためテーブルは存在する）
-- netkeiba は再取得可能なため、weather や trainer 等 horse_past_runs に持たない列は NULL で復元する。

INSERT OR IGNORE INTO races (
    race_id, date, venue, round, day, race_num, surface, distance,
    track_condition, weather, source)
SELECT DISTINCT
    race_id, date, venue, round, day, race_num, surface, distance,
    track_condition, NULL, 'netkeiba'
FROM horse_past_runs;

INSERT OR IGNORE INTO results (
    race_id, finishing_position, status, gate_num, horse_num, horse_name,
    horse_id, jockey, time_seconds, margin, odds, horse_weight,
    weight_change, weight_carried, popularity, source)
SELECT
    race_id, finishing_position, status, gate_num, horse_num, horse_name,
    horse_id, jockey, time_seconds, margin, odds, horse_weight,
    weight_change, weight_carried, popularity, 'netkeiba'
FROM horse_past_runs;
