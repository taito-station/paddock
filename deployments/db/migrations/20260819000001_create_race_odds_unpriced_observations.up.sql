-- 「この券種は netkeiba 上で未発売だと確認できた」という観測を記録する（#632）。
--
-- 背景: read-through の cache-hit 判定 `RaceOdds::is_complete()`（#294 / ADR 0010）は win + 組合せ
-- 5 券種がすべて priced であることを要求する。#621（ADR 0086）以降、未発売の番兵値は
-- `OddsValue::try_from` が弾いて race_odds に入らないため、券種がまるごと未発売の時間帯
-- （前日プリフェッチなど）は当該券種が永久に 0 行 → is_complete が永久 false → read-through が
-- 呼ばれるたびに 6 GET のフルスクレイプが走る。netkeiba 経路には RateGate が無く（ADR 0049）、
-- IP ブロックが本 PJ の最重要運用リスク（ADR 0068）なので、構造で止める。
--
-- なぜ race_odds に番兵行を入れないのか: 番兵は払戻倍率ではなく「未発売」という状態の記録で、
-- ADR 0086 決定 1/3 が「オッズとして拒否する / 既存行は読み出しで無害化する」と決めている。
-- オッズではない観測はオッズのテーブルに入れず、この専用表に置く。
--
-- 行の粒度は (race_id, bet_type)。1 レースあたり最大 5 行（組合せ券種のみ。単勝・複勝は
-- 番兵を持たず〈ADR 0088〉、単複の取得失敗は Err 伝播なので「空＝未発売」と読める状況が無い）。
-- observed_at は UTC rfc3339 文字列（race_odds.fetched_at と同じ TEXT 時刻規約）。
-- 観測は use-case 層が TTL（15 分）付きで解釈する。DB 側で期限切れ行を消す必要は無い
-- （priced が取れた時点で同一トランザクションが DELETE する）。
CREATE TABLE race_odds_unpriced_observations (
    race_id     TEXT NOT NULL,
    bet_type    TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY (race_id, bet_type)
);

-- bet_type の語彙は race_odds / race_odds_snapshots と揃える
-- （20260721000002_add_enum_check_constraints と同じ 7 値）。書き込みは必ず
-- `BetType::*.to_string()`（snake_case Display）を経由するため保存値はこの語彙に限定される。
-- 実際に入りうるのは組合せ 5 券種だが、CHECK は既存 2 表と同じ語彙で揃えて非対称を作らない。
ALTER TABLE race_odds_unpriced_observations
    ADD CONSTRAINT ck_race_odds_unpriced_observations_bet_type
    CHECK (bet_type IN (
        'win', 'place', 'quinella', 'wide', 'exacta', 'trio', 'trifecta'
    ));
