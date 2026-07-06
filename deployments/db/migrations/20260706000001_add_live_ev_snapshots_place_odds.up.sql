-- ライブ EV スナップショットに ◎の複勝オッズを持たせる（#346）。SPA で「単複」を表示するため、
-- 既存の単勝 axis_win_odds に加えて複勝オッズを保存する。複勝は low..high の帯なので 2 列で持つ
-- （axis_win_odds と同じく発走直前でも JRA 未公開なら欠落しうるため nullable）。
--
-- nullable 追加のみなので前方互換: 旧 writer（複勝を書かない persist_live_ev.py / predict-watch）とも
-- 共存でき、read 側は欠落時 NULL を「複勝—」表示に落とす。
ALTER TABLE live_ev_snapshots
    ADD COLUMN axis_place_odds_low  DOUBLE PRECISION,
    ADD COLUMN axis_place_odds_high DOUBLE PRECISION;
