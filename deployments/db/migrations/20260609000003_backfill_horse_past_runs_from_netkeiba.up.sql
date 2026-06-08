-- via:no-schema-check: local SQLite migration. races/results のスキーマは本セッションで
-- create_races / create_results + add_results_columns / add_results_status /
-- add_results_source_horse_id / add_races_source を確認済み。horse_past_runs/horses は同一 migration set で定義。
--
-- 既存の source='netkeiba' 行（races/results）を horses/horse_past_runs へ移送し、
-- 成績テーブルを pdf 確定成績専用にする。horse_id を持たない旧行は移送せず破棄する
-- （netkeiba は fetch-history で再取得可能なため、欠落は次回取得で回復する）。

INSERT OR IGNORE INTO horses (horse_id, horse_name)
SELECT DISTINCT r.horse_id, r.horse_name
FROM results r
INNER JOIN races ra ON ra.race_id = r.race_id
WHERE ra.source = 'netkeiba'
  AND r.horse_id IS NOT NULL;

INSERT OR IGNORE INTO horse_past_runs (
    horse_id, race_id, netkeiba_race_id, date, venue, round, day, race_num,
    surface, distance, track_condition, finishing_position, status, gate_num,
    horse_num, horse_name, jockey, time_seconds, margin, odds, horse_weight,
    weight_change, weight_carried, popularity)
SELECT
    r.horse_id, ra.race_id,
    CASE WHEN ra.race_id LIKE 'nk-%' THEN substr(ra.race_id, 4) ELSE ra.race_id END,
    ra.date, ra.venue, ra.round, ra.day, ra.race_num,
    ra.surface, ra.distance, ra.track_condition,
    r.finishing_position, r.status, r.gate_num, r.horse_num, r.horse_name,
    r.jockey, r.time_seconds, r.margin, r.odds, r.horse_weight,
    r.weight_change, r.weight_carried, r.popularity
FROM results r
INNER JOIN races ra ON ra.race_id = r.race_id
WHERE ra.source = 'netkeiba'
  AND r.horse_id IS NOT NULL;

-- 移送済み netkeiba 行を除去（results を先に、次に races）。
DELETE FROM results WHERE race_id IN (SELECT race_id FROM races WHERE source = 'netkeiba');
DELETE FROM races WHERE source = 'netkeiba';
