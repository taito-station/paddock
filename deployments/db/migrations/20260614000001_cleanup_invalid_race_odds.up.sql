-- via:no-schema-check: race_odds のスキーマは 20260608000001_create_race_odds.up.sql で確認済み
--   (odds REAL NOT NULL / odds_high REAL nullable)。DDL ではなく既知列に対する DELETE のみ。
-- 旧版スクレイパが残した値域違反の残骸行（未公開組合せの 0 埋め等）を一括削除する。これらは
-- find_race_odds が読み取り時に skip する無効データで、残す限り読み取りのたびに skip warn を
-- 出し続ける。恒久的に取り除く(#114)。
--
-- 無効判定は OddsValue::try_from（!is_finite() || value < 1.0 を無効）と境界を揃える:
--   - 下限割れ（odds < 1.0。実在する 0 埋め残骸が該当。-Inf もここで捕捉される）
--   - +Inf（odds = 9e999。SQLite は範囲外の float リテラルを +Inf として扱うため、これで +Inf に一致）
--   - NaN は SQLite の REAL では NOT NULL の odds に格納できない（NULL 化され制約違反）ため考慮不要
-- band 券種は上限 odds_high にも同じ判定を適用する（odds_high NULL は「上限なし」で無効ではない）。
DELETE FROM race_odds
WHERE odds < 1.0
   OR odds = 9e999
   OR (odds_high IS NOT NULL AND (odds_high < 1.0 OR odds_high = 9e999));
