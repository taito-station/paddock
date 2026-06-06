-- via:no-schema-check: SQLite プロジェクトのため INFORMATION_SCHEMA は非対象。`.schema race_cards` / `.schema races` で本セッション中に dump 済み（race_cards に date 無し、races.date は TEXT NOT NULL）。
-- 出馬表(race_cards)に開催日を持たせ、結果がまだ無い未来レースを
-- 日付で予想対象にできるようにする（find_races_by_date が race_cards も参照する）。
ALTER TABLE race_cards ADD COLUMN date TEXT;

-- 既存行は、対応する成績(races)があればその開催日をバックフィルする。
-- 出馬表のみ取り込み済みで結果が無い行は NULL のままとなり、再取り込み時に設定される。
UPDATE race_cards
SET date = (
    SELECT r.date
    FROM races AS r
    WHERE r.race_id = race_cards.race_id
)
WHERE date IS NULL;

-- find_races_by_date は race_cards を date で絞り込むため索引を張る（races(date) と同方針）。
CREATE INDEX idx_race_cards_date ON race_cards(date);
