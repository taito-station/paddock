-- 独立確率モデル（#720）の特徴量の元データ。results と races だけを読み、市場情報の列は選ばない
-- （列の許可リストは pl_features.ALLOWED_COLUMNS と一致させる。テストが両方を突き合わせる）。
-- 出力は TSV（ヘッダ付き）。実行例（読み取りのみ）:
--   limactl shell paddock -- nerdctl exec -i -e PGOPTIONS='-c default_transaction_read_only=on' paddock-postgres psql -U paddock -d paddock -X -q \
--     -c "\copy ($(grep -v '^--' scripts/harness/pl_features_extract.sql | tr '\n' ' ')) to stdout with (format csv, delimiter E'\t', header)" > scripts/harness/data/raw.tsv
SELECT
    x.race_id,
    r.date,
    r.venue,
    r.surface,
    r.distance,
    COALESCE(r.track_condition, '') AS track_condition,
    x.horse_num,
    x.gate_num,
    x.horse_name,
    COALESCE(x.jockey, '') AS jockey,
    COALESCE(x.trainer, '') AS trainer,
    x.weight_carried,
    x.horse_weight,
    x.weight_change,
    x.status,
    x.finishing_position,
    x.time_seconds
FROM results x
JOIN races r USING (race_id)
ORDER BY r.date, x.race_id, x.horse_num
