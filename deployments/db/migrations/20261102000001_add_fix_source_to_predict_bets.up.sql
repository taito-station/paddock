-- 買い目の固定の出所を predict_bets の各行に残す（#795）。
--
-- 記録の 3 経路（web の盤・desktop の盤・CLI の predict）は、同じ固定（朝の選定 → 記録◎ の軸だけ → なし）で
-- 買い目を組む（#776）。どの固定で組んだかを行に残し、REQ-D01-006 の集計で母集団を固定の定義ごとに分ける。
--
-- fix_source          … 'morning'（朝の選定）/ 'recorded_axis'（記録◎ の軸だけ）/ 'none'（固定なし）。
--                         NULL は #795 に未対応のバイナリが書いた行（出所が分からない）。大半は #795 より前の行だが、
--                         migrate の後も再起動していない古い api-server・desktop・predict は NULL を書く（StaleBinary は
--                         warn で起動を続けるため）。区切りは created_at と _sqlx_migrations.installed_on
--                         （version 20261102000001）を比べて付ける（created_at は TEXT〈UTC の RFC3339〉なので
--                         created_at::timestamptz にキャストして比べる）。NULL を CHECK で弾かないのは、古いバイナリの記録を
--                         止めないため。DEFAULT は付けない（新しい行の書き漏れを隠さないため。書き込み側は買い目ありで
--                         出所を必須にしている）。
-- fix_odds_fetched_at … 朝の選定に使った朝のスナップショットの取得時刻（UTC rfc3339）。'morning' のときだけ入る。
--                         morning_selections は先勝ちで延期時は引き直せないので、記録時点の値を写す。
--
-- レース単位の値だが行ごとに持つ（ADR 0013 の predict_race_conditions とは違う形）。#799 の別枠の買い目
-- （1 レースに 2 枠）も同じ列の値で区別するため。
ALTER TABLE predict_bets
    ADD COLUMN fix_source TEXT NULL,
    ADD COLUMN fix_odds_fetched_at TEXT NULL;

ALTER TABLE predict_bets
    ADD CONSTRAINT ck_predict_bets_fix_source
        CHECK (fix_source IN ('morning', 'recorded_axis', 'none')),
    -- fix_source が NULL のときも評価が NULL（＝通過）にならないよう CASE で書く
    ADD CONSTRAINT ck_predict_bets_fix_odds_fetched_at
        CHECK (CASE
                   WHEN fix_source = 'morning' THEN fix_odds_fetched_at IS NOT NULL
                   ELSE fix_odds_fetched_at IS NULL
               END);
