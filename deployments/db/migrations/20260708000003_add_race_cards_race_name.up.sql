-- 表示用のレース名(race_name)を race_cards に保存する（#389）。fetch-card が netkeiba 出馬表の
-- `h1.RaceName`（例「七夕賞」「響灘特別」「3歳上1勝クラス」。グレード表記は含まず、格付けは
-- race_class 側）から埋める。盤・レース一覧のヘッダで重賞・特別戦を名前で識別するために使う。
-- 既存 surface / race_class と同じ TEXT 規約。NULL 許容（PDF 経路・取得失敗時は未設定。過去分の
-- 埋め戻しは不要＝新規取得分から入れば運用上十分）。自由テキストのため CHECK 制約は付けない。
ALTER TABLE race_cards ADD COLUMN IF NOT EXISTS race_name TEXT;
