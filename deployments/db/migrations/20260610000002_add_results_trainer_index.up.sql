-- trainer_stats 集計の WHERE results.trainer = ? を高速化（#74）。
-- jockey の idx_results_jockey と同趣旨。集計は常に trainer 非 NULL を対象にするため、
-- NULL 行を索引から除く部分インデックスにする（現状 results.trainer は全件 NULL）。
CREATE INDEX idx_results_trainer ON results(trainer) WHERE trainer IS NOT NULL;
