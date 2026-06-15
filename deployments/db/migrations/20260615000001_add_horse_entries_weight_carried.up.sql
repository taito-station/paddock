-- 出馬表に負担重量(weight_carried)列を追加（#135）。netkeiba 出馬表から埋める。
-- PDF 経路は未対応のため NULL のまま（確率推定で斤量項なし）。
ALTER TABLE horse_entries ADD COLUMN weight_carried REAL;
