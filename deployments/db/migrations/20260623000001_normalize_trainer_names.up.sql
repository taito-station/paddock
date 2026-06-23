-- horse_entries.trainer の略名（netkeiba が title 属性を省略する仕様）をフルネームに正規化する（#219）。
-- via:no-schema-check: horse_entries・results は上記スクリプトと同セッションで dump 済み

-- Step 1: results 全体から前方一致で一意解決できる略名を正規化する。
--   新人調教師等で一致なし・衝突（2件以上一致）の場合はスキップ。
UPDATE horse_entries he
SET trainer = subq.full_name
FROM (
    SELECT he2.trainer AS abbr, MIN(r.trainer) AS full_name
    FROM horse_entries he2
    INNER JOIN results r
        ON r.trainer LIKE he2.trainer || '%'
        AND r.trainer IS NOT NULL
    WHERE he2.trainer IS NOT NULL
    GROUP BY he2.trainer
    HAVING COUNT(DISTINCT r.trainer) = 1
) subq
WHERE he.trainer = subq.abbr
  AND subq.full_name != he.trainer;

-- Step 2: 同一レース(race_id+horse_num)の results.trainer で直接上書きする。
--   Step 1 で解決できなかった非プレフィックス略名（例:「手塚久」→「手塚貴久」）を補完する。
UPDATE horse_entries he
SET trainer = r.trainer
FROM results r
WHERE r.race_id = he.race_id
  AND r.horse_num = he.horse_num
  AND he.trainer IS NOT NULL
  AND r.trainer IS NOT NULL
  AND r.trainer != he.trainer;
