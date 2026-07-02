-- 近走(horse_past_runs)にレース名とコーナー通過順位を保存する（#272 改善③）。
-- netkeiba 馬個別成績ページの列4(レース名 例「有馬記念(GI)」「1勝クラス」)と
-- 列25(通過順位 例「10-9-5-5」)から取得し、クラス昇降・脚質の新規予測 factor の元データにする。
-- クラス正規化・脚質導出は domain 層で行うため、ここでは生テキストを保持する。
-- 既存行は NULL（backfill は近走再 fetch で埋める）。TEXT/NULL 許容は既存 margin 等と同規約。
ALTER TABLE horse_past_runs ADD COLUMN IF NOT EXISTS race_name TEXT;
ALTER TABLE horse_past_runs ADD COLUMN IF NOT EXISTS corner_positions TEXT;
