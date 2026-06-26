-- 発走時刻(post_time)を race_cards に保存する（#235）。fetch-card が netkeiba 出馬表の
-- RaceData01「HH:MM発走」から取得して埋める。締切前 snapshot の厳密選択（#218）や
-- 締切前フェッチの発火条件に使う。既存 date と同じ TEXT 規約（ここは HH:MM）。NULL 許容
-- （PDF 経路・取得失敗時は未設定）。
ALTER TABLE race_cards ADD COLUMN IF NOT EXISTS post_time TEXT;
