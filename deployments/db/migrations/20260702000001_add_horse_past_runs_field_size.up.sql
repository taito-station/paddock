-- 近走(horse_past_runs)に出走頭数を保存する（#329 Phase1）。
-- netkeiba 馬個別成績ページの列6(頭数 例「15」)から取得し、脚質（先行度）導出で
-- コーナー通過順位を相対化する分母にする（1コーナー3番手は16頭立てと8頭立てで意味が違う）。
-- 既存行は NULL（backfill は近走再 fetch で埋める）。INTEGER/NULL 許容は他の任意列と同規約。
-- 冪等 ADD COLUMN IF NOT EXISTS は共有DBの先行適用列とも整合する（#331 と同方針）。
ALTER TABLE horse_past_runs ADD COLUMN IF NOT EXISTS field_size INTEGER;
