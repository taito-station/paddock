-- race_odds / race_odds_snapshots に値域 CHECK 制約を追加する（#468・#114 再発防止）。
-- アプリ層（save_race_odds: OddsValue::try_from / find_race_odds: warn+skip）だけに頼らず、
-- DB を最終防衛線とする多重防御。
--
-- 【Infinity/NaN の排除について】
-- PostgreSQL は `NaN >= 1.0` および `'Infinity'::float8 >= 1.0` をいずれも TRUE と評価する。
-- そのため下限 (>= 1.0) だけでは +Infinity と NaN を弾けない。
-- ドメイン型 OddsValue::try_from の条件 is_finite() && value >= 1.0 と厳密一致させるため、
-- 上限 `< 'Infinity'::float8` を併用する:
--   * -Infinity は下限 (>= 1.0) で排除
--   * +Infinity は上限 (< 'Infinity') で排除
--   * NaN は PostgreSQL において `NaN < 'Infinity'::float8` が FALSE になるため上限で排除
-- band 整合制約は ドメイン型 PlaceOdds (low <= high) に対応する。
--
-- Postgres には ADD CONSTRAINT IF NOT EXISTS が無いため、再実行可能にするよう
-- 先に DROP CONSTRAINT IF EXISTS してから ADD する（手本 migration と同じパターン）。

-- ---- race_odds ----

ALTER TABLE race_odds DROP CONSTRAINT IF EXISTS ck_race_odds_odds;
ALTER TABLE race_odds ADD CONSTRAINT ck_race_odds_odds
    CHECK (odds >= 1.0 AND odds < 'Infinity'::float8);

ALTER TABLE race_odds DROP CONSTRAINT IF EXISTS ck_race_odds_odds_high;
ALTER TABLE race_odds ADD CONSTRAINT ck_race_odds_odds_high
    CHECK (odds_high IS NULL OR (odds_high >= 1.0 AND odds_high < 'Infinity'::float8));

ALTER TABLE race_odds DROP CONSTRAINT IF EXISTS ck_race_odds_band;
ALTER TABLE race_odds ADD CONSTRAINT ck_race_odds_band
    CHECK (odds_high IS NULL OR odds_high >= odds);

-- ---- race_odds_snapshots ----

ALTER TABLE race_odds_snapshots DROP CONSTRAINT IF EXISTS ck_race_odds_snapshots_odds;
ALTER TABLE race_odds_snapshots ADD CONSTRAINT ck_race_odds_snapshots_odds
    CHECK (odds >= 1.0 AND odds < 'Infinity'::float8);

ALTER TABLE race_odds_snapshots DROP CONSTRAINT IF EXISTS ck_race_odds_snapshots_odds_high;
ALTER TABLE race_odds_snapshots ADD CONSTRAINT ck_race_odds_snapshots_odds_high
    CHECK (odds_high IS NULL OR (odds_high >= 1.0 AND odds_high < 'Infinity'::float8));

ALTER TABLE race_odds_snapshots DROP CONSTRAINT IF EXISTS ck_race_odds_snapshots_band;
ALTER TABLE race_odds_snapshots ADD CONSTRAINT ck_race_odds_snapshots_band
    CHECK (odds_high IS NULL OR odds_high >= odds);
