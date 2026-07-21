-- 列挙的 TEXT 列（bet_type / verdict）に列挙 CHECK 制約を追加する（#472）。
-- 新設列は CHECK を付ける方針（#345 ck_race_cards_race_class / #344 ck_live_ev_snapshots_roughness）
-- に転換済みだが、旧設テーブルの列挙 TEXT 列は CHECK が無く非対称だった。アプリ層のガード
-- （save/find 経路）だけに頼らず、タイポ列挙値・想定外ラベルを DB の最終防衛線で弾く多重防御。
--
-- 【本 migration の scope】列挙 CHECK のみ。孤児行を防ぐ FK（race_id → races/race_cards の
-- 二系統設計のため無条件には貼れない）は別途検討で、本 migration では扱わない。
--
-- 【bet_type の許容集合＝BetType の snake_case Display 7 値】
--   win / place / quinella / wide / exacta / trio / trifecta
-- race_odds / race_odds_snapshots への書き込みは必ず `OddsRow::{win,place,...}`（use-case
-- repository.rs）を経由し、いずれも `BetType::*.to_string()`（domain odds/bet_type.rs の
-- `#[strum(serialize_all = "snake_case")]` Display）で bet_type を詰める。よって保存値は
-- この 7 値に限定される（読み側 find_race_odds は BetType::try_from で日本語別名も受けるが、
-- それは「読み取り耐性」であって保存値の語彙ではない）。golden DB の実測でも両テーブルの
-- DISTINCT bet_type はこの 7 値と完全一致し、既存行は本 CHECK に違反しない。
--
-- ※ predict_bets / prediction_bets の bet_type は本 migration の対象外。predict_bets は同じ
--   snake_case 語彙（BetCombination::type_label）だが issue #472 の証拠が指す race_odds 系では
--   ない。prediction_bets は ingest-predictions が JSON の bet_type を無変換で保存する自由記述
--   （日本語ラベル 単勝/複勝/… もあり得る）で語彙が制御されておらず、CHECK 化は既存/将来の
--   正当行を弾く恐れがあるため意図的に除外する（FK 同様、別途検討）。
--
-- 【verdict の許容集合＝bet / skip】
-- live_ev_snapshots.verdict へ DB 保存する経路は Rust 1 本のみで、ROI ゲートで 'bet'/'skip' の
-- 二値しか出さない:
--   * src/apps/predict-watch/src/snapshot.rs: `if ev.roi >= 1.0 { "bet" } else { "skip" }`
-- （scripts/predict-check/live_ev.py も `"bet" if roi >= 100 else "skip"` と同語彙を emit するが、
--  自身の docstring どおり DB 永続化は退役済みで live_ev_snapshots を書かない＝許容集合には影響しない。
--  domain の Verdict enum = Strong/Weak/Neutral は factor 説明用の別物で、この列には保存しない）。
-- golden DB の DISTINCT verdict も 'skip' のみ（'bet' も有効）で、既存行は本 CHECK に違反しない。
--
-- 【ロックについて】NOT VALID を付けない ADD CONSTRAINT は ACCESS EXCLUSIVE ロック下で既存行を
-- 全スキャン検証する。race_odds_snapshots は追記テーブルで規模があるが、検証スキャンは適用時一度
-- きり・数秒規模で、golden に違反行は無いため全行が通る。現規模では二段階（NOT VALID → VALIDATE）
-- にする必要はないと判断し、最小構成の直接追加とする（#468 の odds 値域 CHECK と同方針）。
--
-- Postgres には ADD CONSTRAINT IF NOT EXISTS が無いため、再実行可能にするよう先に
-- DROP CONSTRAINT IF EXISTS してから ADD する（#344/#345/#468 の CHECK migration と同パターン）。

-- ---- race_odds.bet_type ----

ALTER TABLE race_odds DROP CONSTRAINT IF EXISTS ck_race_odds_bet_type;
ALTER TABLE race_odds ADD CONSTRAINT ck_race_odds_bet_type
    CHECK (bet_type IN (
        'win', 'place', 'quinella', 'wide', 'exacta', 'trio', 'trifecta'
    ));

-- ---- race_odds_snapshots.bet_type ----

ALTER TABLE race_odds_snapshots DROP CONSTRAINT IF EXISTS ck_race_odds_snapshots_bet_type;
ALTER TABLE race_odds_snapshots ADD CONSTRAINT ck_race_odds_snapshots_bet_type
    CHECK (bet_type IN (
        'win', 'place', 'quinella', 'wide', 'exacta', 'trio', 'trifecta'
    ));

-- ---- live_ev_snapshots.verdict ----

ALTER TABLE live_ev_snapshots DROP CONSTRAINT IF EXISTS ck_live_ev_snapshots_verdict;
ALTER TABLE live_ev_snapshots ADD CONSTRAINT ck_live_ev_snapshots_verdict
    CHECK (verdict IN ('bet', 'skip'));
