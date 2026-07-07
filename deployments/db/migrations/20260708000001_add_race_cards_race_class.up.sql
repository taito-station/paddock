-- レースの格付け／条件クラス(race_class)を race_cards に保存する（#345）。fetch-card が
-- netkeiba 出馬表の `<title>` グレード表記と RaceData02 条件から判定して埋める。G1 裏レース
-- （G1 開催日・別場の非重賞）の常時通知や、新馬・クラス別の挙動分析に使う。既存 surface と
-- 同じ TEXT 規約。NULL 許容（PDF 経路・取得失敗・判定不能時は未設定）。値はアプリ側 enum
-- `RaceClass::as_str` の安定スラッグと同期する。
ALTER TABLE race_cards ADD COLUMN IF NOT EXISTS race_class TEXT;

-- Postgres には ADD CONSTRAINT IF NOT EXISTS が無いため、列の IF NOT EXISTS と粒度を揃えて
-- 再実行可能にするよう、先に DROP CONSTRAINT IF EXISTS してから ADD する。
ALTER TABLE race_cards DROP CONSTRAINT IF EXISTS ck_race_cards_race_class;
ALTER TABLE race_cards ADD CONSTRAINT ck_race_cards_race_class
    CHECK (race_class IS NULL OR race_class IN (
        'g1', 'g2', 'g3', 'listed', 'open', 'win3', 'win2', 'win1', 'maiden', 'newcomer'
    ));
