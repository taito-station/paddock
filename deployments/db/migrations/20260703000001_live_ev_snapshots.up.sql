-- ライブ EV 監視サイクルの評価結果（張る/見送り判定＋買い目伝票）を時系列アーカイブする
-- append-only テーブル（#260, ADR 0064）。`live_ev.py --emit-json` の出力を refresh_ev.sh が
-- サイクルごとに persist し、read API `GET /api/live/{date}` が race ごと最新サイクル＋直前を返す。
--
-- 時刻・日付は既存テーブル（races.date / race_odds_snapshots.fetched_at）と同じ TEXT 規約に揃える:
--   date        … 'YYYY-MM-DD'（races.date と突合可能にするため TEXT）
--   captured_at … UTC rfc3339 文字列（辞書順=時刻順。race_odds_snapshots.fetched_at と同規約）で、
--                 「最新サイクル = MAX(captured_at)」「直前 = 2番目」を辞書順比較で導出する。
--   post_time   … race_cards.post_time（netkeiba 由来文字列）をそのまま写す。nullable。
CREATE TABLE IF NOT EXISTS live_ev_snapshots (
    id             BIGSERIAL PRIMARY KEY,
    date           TEXT NOT NULL,
    race_id        TEXT NOT NULL,
    venue          TEXT NOT NULL,
    race_no        BIGINT NOT NULL,
    post_time      TEXT,
    captured_at    TEXT NOT NULL,
    verdict        TEXT NOT NULL,             -- 'bet'（ROI>=100）/ 'skip'（-EV）
    roi            DOUBLE PRECISION NOT NULL, -- 全3券種 ROI[%]
    konsen         BOOLEAN NOT NULL,
    axis           BIGINT NOT NULL,           -- ◎馬番（model 勝率最上位）
    axis_prob      DOUBLE PRECISION NOT NULL, -- ◎の model 勝率[%]
    axis_win_odds  DOUBLE PRECISION,          -- ◎の単勝オッズ（欠落時 NULL）
    odds_missing   BOOLEAN NOT NULL,          -- 一部買い目のオッズ欠落（ROI 過小評価の可能性）
    slip           JSONB NOT NULL,            -- 買い目伝票（券種×方式レイヤーの leg 配列）
    raw            JSONB NOT NULL,            -- emit-json の races[] 要素 1 件（原本・後方互換）
    -- 同一サイクルの再実行（cron 二重発火・手動再走）を冪等にするための一意キー。
    -- captured_at はサイクル論理境界時刻を persist が全レース同一値で割り当てる（refresh_ev.sh）。
    CONSTRAINT uq_live_ev_snapshots UNIQUE (race_id, captured_at)
);

-- date で 1 開催日を引き、race ごとに最新（＋直前）サイクルを取り出すための index。
-- captured_at DESC で「最新」「2番目に新しい」を効率よく取得する（フリップ算出に直前が要る）。
CREATE INDEX IF NOT EXISTS idx_live_ev_snapshots_date_race
    ON live_ev_snapshots (date, race_id, captured_at DESC);
