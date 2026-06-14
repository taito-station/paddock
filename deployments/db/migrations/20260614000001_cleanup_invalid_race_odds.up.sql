-- via:no-schema-check: race_odds のスキーマは 20260608000001_create_race_odds.up.sql で確認済み
--   (odds REAL NOT NULL / odds_high REAL nullable)。DDL ではなく既知列に対する DELETE のみ。
-- 旧版スクレイパが残した値域違反の残骸行（未公開組合せの 0 埋め等。odds < 1.0、または
-- band 券種の上限 odds_high < 1.0）を一括削除する。これらは find_race_odds が読み取り時に
-- skip する無効データで、残す限り読み取りのたびに skip warn を出し続ける。恒久的に取り除く(#114)。
DELETE FROM race_odds
WHERE odds < 1.0
   OR (odds_high IS NOT NULL AND odds_high < 1.0);
