# QA: 券種まるごと未発売時の再スクレイプ抑止（#632）

対象: [ADR 0089](../original-docs/0089-unpriced-bet-type-observation.md) の設計判断。
着手前にユーザーへ提示し、回答を得てから実装した論点を記録する。

---

## Q1. 再スクレイプを止める方式は「券種ごとの未発売観測」か「レース単位の debounce」か

**推奨**: 券種ごとの未発売観測。

**理由**: レース単位の一律 debounce は実装が最小で ADR 0068 に語彙もあるが、
**未発売と一過性の取得失敗を区別しない**。区別しないと exotic の一過性失敗まで N 分待たされ、
#294 が作った自己修復（欠けた券種を次回すぐ埋める）が鈍る。#632 の要件 1 が求めている区別も
満たさない。

**回答: 確定。券種ごとの未発売観測を記録する。**

反映先: ADR 0089 決定 1・4、却下案「レース単位の一律 scrape debounce」。
担保: `missing_without_mark_still_rescrapes_every_call`（観測の無い欠落は毎回取り直す）。

---

## Q2. 未発売観測を信用する有効期間（TTL）は

**推奨**: 15 分。

**理由**: TTL はそのまま「発売開始に気づくまでの最大遅れ」になる。`paddock-odds-collect` の
収集間隔と同じ刻みにすると運用上の刻みが揃う。前日プリフェッチ帯の再スクレイプは 1 レース
あたり最大 4 回/時に収まり、当日の発売開始にも十分速く追従する。発走直前の鮮度は
`predict-watch`（read-through を通らず毎回再取得・#257）が担保しているので、この遅れは
判断に影響しない。

**回答: 確定。15 分。**

反映先: ADR 0089 決定 5、`UNPRICED_OBSERVATION_TTL`。
担保: `is_cache_fresh_ignores_observations_older_than_ttl` /
`rescrapes_once_unpriced_mark_is_stale`。

---

## Q3. issue 要件 3「再取得回数を改善前後で実測」をどこまで PR に含めるか

**推奨**: 決定的な計測（FakeScraper のスクレイプ回数）を PR に含め、前日プリフェッチ〜当日発売開始を
またぐ実地計測は次開催で follow-up。

**理由**: 実地計測は開催日をまたぐ時間が要り、PR の着地を 1 週間遅らせる。一方で「N 回呼ぶと
N 回スクレイプしていたものが 1 回になる」は決定的テストで恒久的に固定できる。

**回答: 確定。決定的計測を PR に含め、実地計測は次開催（2026-08-22/23）で follow-up issue。**

反映先: ADR 0089「影響」、`unpriced_bet_types_are_not_rescraped_within_ttl`。

補足: 変異検査で「修正前の挙動（`is_complete` のみで判定）」に差し戻したところ、当該テストは
`left: 5, right: 1` で失敗した。**テストが実際にこの欠陥を捕まえている**ことを確認済み。

---

## Q4. 未発売の判定は「全行が番兵か」か「priced が 0 件か」か

質問票には出していないが、実装中に判断が要った点。**計画では「全行が番兵」と書いていたが
「priced が 0 件」に変えた。**

**理由**: 実地のワイド未発売行は netkeiba が `["9999.9", "0.0", "--"]` の形で返す
（`QA-odds-sentinel-621.md` Q2）。相方の `0.0` は番兵ではなく**値域違反**として弾かれるため、
「全行が番兵か」で判定するとワイドの未発売を取りこぼす。「取得に成功したのに priced が 0 件か」なら
両方拾える。

副産物として、JRA がそもそも売らない極小頭数レースの券種（行が 0 件で返る）も未発売として拾い、
ADR 0010 が「許容する」としていた毎回再スクレイプも閉じる。

**回答: priced が 0 件かで判定する。**

反映先: ADR 0089 決定 3。
担保: `real_wide_unpriced_shape_is_observed_as_unpriced`（番兵基準だと落ちる形の回帰ガード）。

---

## Q5. `RaceOdds::is_complete()` の意味を変えるか

**回答: 変えない。** cache-hit の判断は use-case 層に置く。

**理由**: `find_race_odds_morning` の「朝時点＝最初にフル盤が成立した snapshot」がこの意味に
依存しており（`rest-api-read.md`）、ADR 0088 が `find_race_odds_morning` は挙動不変と明言している。
意味を変えると board の朝↔現比較が静かにズレる。

欠落券種の列挙は `RaceOdds::missing_bet_types()` に集約し、`is_complete()` はそれを使って実装する
（判定基準を 2 箇所に持たない・ADR 0064 の second source 回避）。

反映先: ADR 0089 決定 6。
担保: 既存の `morning_returns_earliest_complete_snapshot_with_bounds` /
`incomplete_snapshot_converges_to_complete_via_upsert` が緑のまま。

---

## Q6. `FetchedExoticOdds::default()` の扱い

実装中に見つけた落とし穴。`failed` が空の `default()` を `assemble_netkeiba` に渡すと、
**全券種が「取得成功して 0 行」＝未発売と誤判定される**。単複のみ取得（odds-collect）と
組合せ取得の丸ごと失敗はこの形になる。

**回答: 観測していない経路は `all_exotic_failed()` を使う。**

反映先: ADR 0089「影響」。
担保: `win_place_only_scrape_observes_no_unpriced_bet_type` /
`failed_bet_type_is_not_observed_as_unpriced`。
