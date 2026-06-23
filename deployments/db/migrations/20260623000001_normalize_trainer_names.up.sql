-- horse_entries.trainer の略名（netkeiba が title 属性を省略する仕様）をフルネームに正規化する（#219）。
-- 処理順序: ① 全 results 前方一致（大多数をカバー）→ ② 同一レース直接上書き（非プレフィックス略名を補完）。
-- Rust の normalize_trainer_names とは処理順が逆だが、② が ① を上書きする構造のため最終結果は同値。

-- ① 全 results から前方一致で一意解決できる略名を正規化する（大多数のケースをカバー）。
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

-- ② 同一レース(race_id+horse_num)の results.trainer で直接上書きする。
--   ① で解決できなかった非プレフィックス略名（例:「手塚久」→「手塚貴久」）を補完する。
--   ① の一致が誤解決だった行も正しいフルネームで上書きされる。
UPDATE horse_entries he
SET trainer = r.trainer
FROM results r
WHERE r.race_id = he.race_id
  AND r.horse_num = he.horse_num
  AND he.trainer IS NOT NULL
  AND r.trainer IS NOT NULL
  AND r.trainer != he.trainer;
