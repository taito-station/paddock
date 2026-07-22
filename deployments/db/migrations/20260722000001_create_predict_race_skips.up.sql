-- 予想セッションで「このレースを見送り（スキップ）」と記録した痕跡を永続化する（#481）。
--
-- 従来スキップは outcome を空 bets で POST するだけで DB に何も残さず、リロード後に web 盤が
-- 「未処理（記録する導線）」に戻っていた（処理済み/未処理の判別不能で二度見が発生）。買い目を
-- 持たない「見送り済み」を per-race で記録し、session サマリで再訪時にバッジ表示できるようにする。
--
-- 隣接する predict_race_conditions（馬場入力の per-race 記録）と同じ規約に揃える:
--   session_date … predict_sessions(date) への FK。セッション削除で連鎖削除する。
--   created_at   … UTC rfc3339 文字列（既存 predict_* テーブルの TEXT 時刻規約）。
-- 買い目ありの記録は predict_bets 側に残るため、この表は「買い目なしで処理済み」専用とする。
CREATE TABLE predict_race_skips (
    session_date TEXT NOT NULL REFERENCES predict_sessions(date) ON DELETE CASCADE,
    race_id      TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    PRIMARY KEY (session_date, race_id)
);
