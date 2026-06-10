-- trainer_stats 集計の WHERE results.trainer = ? を高速化（#74）。
-- jockey の idx_results_jockey と同趣旨。
CREATE INDEX idx_results_trainer ON results(trainer);
