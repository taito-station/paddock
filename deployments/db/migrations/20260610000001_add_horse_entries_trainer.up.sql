-- 出馬表に調教師(trainer)列を追加（#74）。netkeiba 出馬表から埋める。
-- PDF 経路は未対応のため NULL のまま（確率推定で trainer 項なし）。
ALTER TABLE horse_entries ADD COLUMN trainer TEXT;
