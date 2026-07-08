-- 荒れ度(roughness)を live_ev_snapshots に保存する（#344）。predict-watch が 1 レース評価時に
-- 純モデル勝率分布の正規化シャノンエントロピー [0,1] を算出して埋める（0=堅い〜1=荒れ）。
-- ライブ日次ボードの「荒れ度チップ」表示に使う。ROI(=期待値)とは別軸(=分布の乱れ)。odds 非依存。
-- NULL 許容（本 migration 以前の行・算出不能時は未設定）。read 側は Option で扱う。
ALTER TABLE live_ev_snapshots ADD COLUMN IF NOT EXISTS roughness DOUBLE PRECISION;

ALTER TABLE live_ev_snapshots DROP CONSTRAINT IF EXISTS ck_live_ev_snapshots_roughness;
ALTER TABLE live_ev_snapshots ADD CONSTRAINT ck_live_ev_snapshots_roughness
    CHECK (roughness IS NULL OR (roughness >= 0.0 AND roughness <= 1.0));
