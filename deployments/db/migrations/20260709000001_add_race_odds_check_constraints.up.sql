-- race_odds / race_odds_snapshots に odds 値域 CHECK 制約を追加する（#468・#114 再発防止）。
-- アプリ層（save_race_odds: OddsValue::try_from / find_race_odds: warn+skip）だけに頼らず、
-- DB を最終防衛線とする多重防御。
--
-- 【Infinity/NaN の排除について】
-- PostgreSQL は float の順序付けを IEEE754 と変えており、NaN を「全ての実数・±Infinity より
-- 大きい最大値」として扱う。そのため `NaN >= 1.0` も `'Infinity'::float8 >= 1.0` もいずれも
-- TRUE と評価され、下限 (>= 1.0) だけでは +Infinity と NaN を弾けない。
-- ドメイン型 OddsValue::try_from の条件 is_finite() && value >= 1.0 と厳密一致させるため、
-- 上限 `< 'Infinity'::float8` を併用する:
--   * -Infinity は下限 (>= 1.0) で排除
--   * +Infinity は上限 (< 'Infinity') で排除
--   * NaN は PostgreSQL において `NaN < 'Infinity'::float8` が FALSE になるため上限で排除
--
-- 【band 整合 (odds_high >= odds) は意図的に DB 制約化しない】
-- 下限・上限とも値域内だが low>high の band 行は、保存側ガードが値域のみ見るため**保存され**、
-- 読み取り側 parse_band が構造不正として Err（skip せず stop）で早期検知する——という意図的な
-- 非対称設計（#114 / CLAUDE.md・回帰テスト save_keeps_inverted_band_which_read_rejects）が既にある。
-- ここで band CHECK を足すと保存自体を弾いてこの設計を上書きしてしまうため、本 migration は
-- 「値域（有限・>=1.0）」のみを DB 制約化し、band 整合は既存の read 側検知に委ねる。
--
-- 【ロックについて】NOT VALID を付けない ADD CONSTRAINT は ACCESS EXCLUSIVE ロック下で既存行を
-- 全スキャン検証する。race_odds_snapshots は追記テーブルで規模があるが、検証スキャンは一度きり
-- （適用時のみ）かつ数秒規模で、golden に値域違反行は無い（#114）ため既存行は全て通る。現規模では
-- 二段階（NOT VALID → VALIDATE CONSTRAINT）にする必要はないと判断し、最小構成の直接追加とする。
--
-- Postgres には ADD CONSTRAINT IF NOT EXISTS が無いため、再実行可能にするよう
-- 先に DROP CONSTRAINT IF EXISTS してから ADD する（issue #344/#345 の CHECK 制約 migration
-- = 20260708000001_add_race_cards_race_class / 20260708000002_add_live_ev_snapshots_roughness と同パターン）。

-- ---- race_odds ----

ALTER TABLE race_odds DROP CONSTRAINT IF EXISTS ck_race_odds_odds_range;
ALTER TABLE race_odds ADD CONSTRAINT ck_race_odds_odds_range
    CHECK (odds >= 1.0 AND odds < 'Infinity'::float8);

ALTER TABLE race_odds DROP CONSTRAINT IF EXISTS ck_race_odds_odds_high_range;
ALTER TABLE race_odds ADD CONSTRAINT ck_race_odds_odds_high_range
    CHECK (odds_high IS NULL OR (odds_high >= 1.0 AND odds_high < 'Infinity'::float8));

-- ---- race_odds_snapshots ----

ALTER TABLE race_odds_snapshots DROP CONSTRAINT IF EXISTS ck_race_odds_snapshots_odds_range;
ALTER TABLE race_odds_snapshots ADD CONSTRAINT ck_race_odds_snapshots_odds_range
    CHECK (odds >= 1.0 AND odds < 'Infinity'::float8);

ALTER TABLE race_odds_snapshots DROP CONSTRAINT IF EXISTS ck_race_odds_snapshots_odds_high_range;
ALTER TABLE race_odds_snapshots ADD CONSTRAINT ck_race_odds_snapshots_odds_high_range
    CHECK (odds_high IS NULL OR (odds_high >= 1.0 AND odds_high < 'Infinity'::float8));
