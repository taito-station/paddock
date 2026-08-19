# 0089. 未発売と確認できた券種を観測として記録し、read-through の cache-hit に織り込む

## ステータス

承認済み（[#632](https://github.com/taito-station/paddock/issues/632)）。

**[ADR 0010](0010-persist-and-reference-odds.md) の「後日談（#294）」で定めた cache-hit 規則を
部分 supersede する**: 「不完全なスナップショットは cache-miss として再スクレイプする」という
原則は維持したまま、**「未発売と確認できた券種の欠落」を不完全さから除外する**。
`RaceOdds::is_complete()` の意味（priced な行が全券種そろっているか）は変えない。ADR 0010 自体は
書き換えない。

[ADR 0086](0086-netkeiba-unpriced-sentinel-is-not-odds.md) の決定 1/3（番兵はオッズではない・
`race_odds` に入れない）と [ADR 0088](0088-bet-type-scoped-unpriced-sentinels.md) の券種スコープ
判定はそのまま維持し、本 ADR はその上に乗る。

## コンテキスト

ADR 0010 の read-through は、cache-hit 判定を `RaceOdds::is_complete()`（win + 組合せ 5 券種が
すべて priced）に置いている（#294）。#621（ADR 0086）以降、netkeiba の未発売番兵は
`OddsValue::try_from` が弾いて `race_odds` に入らない。

その結果、**券種がまるごと未発売の時間帯では当該券種が永久に 0 行**になり、`is_complete()` は
永久に false になる。read-through は呼ばれるたびに cache-miss と判定し、1 レースあたり 6 GET の
フルスクレイプを打ち直す。

これは ADR 0086 が「影響」節で予告していた副作用そのものである:

> （負荷）券種がまるごと未発売の時間帯は `trio` 等が空になり complete に届かず、read-through が
> 呼ばれるたびに再スクレイプする。**netkeiba へのペーシング規律に触れる**ので、前日プリフェッチを
> 多用する運用では取得回数を見ておく

ADR 0010 も同じ穴を「極小頭数レース」の文脈で認識し、**呼び出しは 1 レース 1 回程度という前提で
許容**していた:

> なお JRA が一部券種を発売しない極小頭数レースでは is_complete が常に false になり read-through で
> 毎回再スクレイプするが、UPSERT で行は肥大せず呼び出しも 1 レース 1 回程度のため許容する。

**この前提はもう成り立たない。** `predict --overview`（#551）は完了済みセッションでも何度でも
再実行できる設計で、36 鞍の開催日なら 1 回の実行で 216 GET。朝に数回流せば四桁になる。
netkeiba 経路には JRA 側の `RateGate`（ADR 0021/0029）に相当する帯域制御が無く（ADR 0049 が
「バルク fetch のレート制御は別系統」と明記）、netkeiba に対する規律は
**「無駄打ちを構造的に止める」**型——`post_time` 前・確定済みは取得しない gating、
直近取得の debounce（ADR 0068）、全レース確定でポーリング停止——に置かれている。
IP ブロックは本 PJ の最重要運用リスク（ADR 0068）なので、構造で止める。

方向性は ADR 0088 が既に置いている（「関連」節）:

> #632 / #633（未発売の観測記録。**本 ADR の券種付き判定 API の上に乗る**）

### 情報は作れるのに捨てられている

`assemble_netkeiba`（`src/interface/netkeiba-scraper/src/scraper.rs`）は券種ごとに独立したループで
`OddsValue::try_from` を通し、**失敗した行を黙って捨てる**。このとき「netkeiba は行を返したが
1 つも priced にならなかった」＝**未発売と確認できた**という事実が作れるのに、戻り値
`RaceOdds` に載せる場所が無いため失われていた。

一方 `fetch_one_exotic` は取得失敗を空 Vec に畳んでいた（券種単位のベストエフォート・#102）ため、
**「失敗して空」と「未発売で空」が区別できなかった**。この区別が本 ADR の要である。

## 決定

1. **「未発売と確認できた券種」を観測として記録し、cache-hit 判定で欠落から差し引く。**
   保存済みオッズの欠落券種が、TTL 内の未発売観測にすべて収まるなら cache-hit（再スクレイプしない）。

2. **観測は専用テーブル `race_odds_unpriced_observations(race_id, bet_type, observed_at)` に置く。**
   番兵行を `race_odds` に入れて complete を満たさせる方向は採らない——ADR 0086 決定 1/3
   （番兵はオッズではない・読み出しで無害化する）と正面から矛盾する。「オッズではない観測」は
   オッズのテーブルに入れない。

3. **未発売の判定は「取得に成功したのに priced が 0 件か」で行う。**「全行が番兵か」では判定しない。
   実地のワイド未発売行は `["9999.9", "0.0", "--"]` の形（`QA-odds-sentinel-621.md` Q2）で、
   相方の `0.0` は番兵ではなく**値域違反**として弾かれるため、番兵の有無だけを見るとワイドの
   未発売を取りこぼす。この判定は副産物として、JRA がそもそも売らない極小頭数レースの券種
   （行が 0 件で返る）も拾い、ADR 0010 が「許容する」としていた毎回再スクレイプを閉じる。

4. **観測できた券種だけを対象にする。** `fetch_one_exotic` は取得に成功した券種を呼び出し側へ伝え
   （`FetchedExoticOdds::observed`）、**そこに載っていない券種は未発売マークを作らない**。
   取得失敗は「分からない」であって「売っていない」ではなく、**次回そのまま取り直させる必要がある**
   ——ここを混ぜると #294 の自己修復（exotic の一過性失敗を次回すぐ埋める）が鈍る。

   「失敗した券種の集合」ではなく「観測できた券種の集合」を持つのは、**`Default`（空集合）が
   最も安全な解釈＝「何も観測していない」になるようにするため**。逆向きだと既定値が
   「全券種を観測して全部空だった」＝全券種未発売、という最も危険な解釈になり、
   単複のみ取得（odds-collect）や組合せ取得の丸ごと失敗で誤マークを生む。

5. **観測の TTL は 15 分。** これは同時に「発売開始に気づくまでの最大遅れ」でもある。
   `paddock-odds-collect` の収集間隔と同じ刻みにした。前日プリフェッチ帯の再スクレイプは
   1 レースあたり最大 4 回/時に収まる。発走直前の鮮度はこの TTL では損なわれない
   ——`predict-watch` は read-through を通らず毎回再スクレイプする（#257）。

6. **観測の解釈は 2 つの向きで安全側に倒す。** どちらも「迷ったら取り直す」:
   - **単勝・複勝の観測は cache-hit 判定で無視する。** 本番の書き込み経路は組合せ 5 券種しか
     記録しないが、DB の CHECK は語彙統一のため 7 値を許している。仮に `win` の観測行が入ると
     単勝の欠落を免除してしまい、`race_odds()` が win 空のスナップショットを cache-hit で返す
     （fetch-card が degraded 分岐で明示的に避けている「オッズ有り・win 無し」の再現）。
   - **未来時刻の観測は stale 扱いにする。** 時計のズレやダンプ復元で `observed_at` が未来に
     なると、単純な差分比較では無条件に fresh と判定されて再取得が止まる。gateway が
     「壊れた `observed_at` は読み飛ばす」としているのと向きを揃える。

7. **`RaceOdds::is_complete()` の意味は変えない。** 「priced な行が全券種そろっているか」のまま
   据え置き、cache-hit の判断は use-case 層（`OddsInteractor::race_odds`）が持つ。
   `find_race_odds_morning` の「朝時点＝最初にフル盤が成立した snapshot」（ADR 0088 が
   「挙動不変」と明言・`rest-api-read.md`）がこの意味に依存しているため。
   欠落券種の列挙は `RaceOdds::missing_bet_types()` に集約し、`is_complete()` はそれを使って
   実装する（判定基準の second source を作らない・ADR 0064）。

8. **priced が取れた券種のマークは同一トランザクションで削除する。** 発売が始まったのに
   「未発売」の観測が残ると、次にその券種が一過性失敗で欠けたとき誤って cache-hit してしまう。
   **`save_race_odds` に失敗した回は観測を記録しない**——古いスナップショットに新しいマークが
   付くと、TTL のあいだ古い値を cache-hit で返し続ける（「迷ったら取り直す」に倒す）。

9. **観測は `predict-watch` 経路（`refresh_race_odds`）でも記録する。** 監視側は cache を見ないが、
   発売開始を最初に観測するのは 5 分毎に回る監視であることが多く、そこでマークを消しておくと
   read-through 側も次回すぐ取り直せる。

## 却下した案

- **レース単位の一律 scrape debounce**（「直近 N 分に取得済みなら再取得しない」）。実装は最小で、
  ADR 0068 に語彙もある。しかし**未発売と一過性失敗を区別しない**ため、exotic の一過性失敗まで
  N 分待たされ、#294 の自己修復が鈍る。#632 の要件 1 が求めた区別も満たさない。
- **番兵行を `race_odds` に保存して `is_complete()` を満たさせる。** ADR 0086 決定 1/3 と正面から
  矛盾する。番兵は払戻倍率ではないので、保存すれば EV 側で 1 点 3 桁の参考 ROI を作る危険が戻る。
- **`is_complete()` から組合せ券種を外す**（ADR 0010 が `place` を外したのと同じ手）。`place` は
  「netkeiba が win と同梱で返すが未公開があり得る」ため外せたが、組合せ券種は買い目の本体
  （三連複・ワイド・馬連）であり、欠落を検知しないと部分スナップショットで買い目を組んでしまう。
  #294 が塞いだ穴を開け直すことになる。
- **TTL を置かず「一度未発売と観測したら当日は取り直さない」。** 発売開始を永久に検知できない。
- **`RaceOdds` に `unpriced` フィールドを足してスクレイパ経路とDB復元経路で共用する。**
  `find_race_odds` 経由では常に空になり、**同じ型が経路によって別の意味を持つ**。ポートの戻り値を
  `ScrapedOdds { odds, unpriced }` に変えるほうが正直で、変更もスクレイパ実装・ポート・
  `OddsInteractor` の 2 箇所・テスト fake に閉じる（`predict-watch` / `odds-collect` / `fetch-card`
  は Interactor 越しなので無変更）。

## 影響

- **read-through の取得回数が減る。** 券種まるごと未発売のレースで `race_odds()` を N 回呼んだとき、
  スクレイプは修正前 N 回 → 修正後 1 回（TTL 内）。決定的な回帰テスト
  （`unpriced_bet_types_are_not_rescraped_within_ttl`）で N=5 のケースを固定した。
  **前日プリフェッチ〜当日発売開始をまたぐ実地の計測は #632 の follow-up として別途行う**
  ——ADR 0086 が「取得回数を見ておく」としながら測定値を残していない穴もそこで埋める。

- **効くのは「単勝は取れるが組合せ券種が未発売」の状態**。次の 2 つは**本 ADR では改善しない**
  ので、期待値を取り違えないこと:
  - **全券種が空のレース**（単勝すら未公開。前日の早い時間帯など）。`race_odds()` は
    `Ok(None)` を返して何も保存せず観測も残さないため、**呼ぶたびに従来どおりフルスクレイプ
    される**。レース単位の「まだ何も公開されていない」観測を持たせるかは別途判断する
    （券種ごとの発売有無を確認できたわけではないので、本 ADR の観測とは意味が違う）。
  - **`fetch-card`（前日プリフェッチの実行体）自身**。`assemble_netkeiba` を通らず独自に
    オッズを組み立てるため観測を記録しない。したがって prefetch 後の**最初の read-through は
    レースごとに 1 回フルスクレイプを払う**。必要な情報はこの経路にも揃っているので、
    記録するかは follow-up で判断する。

- **恒久的に未発売の券種があるレースでは、priced 済みオッズが最大 TTL ぶん古くなりうる。**
  JRA が三連単等を売らない極小頭数レースでは、修正前は毎回再スクレイプしていたため
  read-through が返す単勝オッズは常にライブだった。修正後は最大 15 分前の保存値を返す。
  判断に使う発走直前オッズは `predict-watch`（read-through を通らない・#257）が担保するので
  実害は無いが、`paddock-predict` 本体の表示値はこのぶん遅れる。

- **観測表に retention は設けない。** 行は `(race_id, bet_type)` で 1 レース最大 5 行、
  priced が取れた時点で削除される。恒久未発売の券種だけが残るが、`race_odds_snapshots`
  （15 分毎の append・#234 で purge を用意した）とは桁が違うので purge は要らないと判断した。
  必要になれば既存の purge に相乗りさせる。
- **発売開始の反映が最大 15 分遅れる**（read-through 経路のみ）。判断に使う発走直前オッズは
  `predict-watch` が毎回再取得するので影響しない。
- **`OddsScraper::scrape` の戻り値型が変わる**（`RaceOdds` → `ScrapedOdds`）。実装・fake の
  追従が要る。`scrape_win_place` は `RaceOdds` のまま（単複だけの観測は組合せ券種について
  何も言わないため、この経路は未発売マークを作らない）。
- **`FetchedExoticOdds::default()` は安全**（`observed` が空＝「何も観測していない」）。
  単複のみ取得・組合せ取得の丸ごと失敗はこの既定値をそのまま使える。回帰テスト
  `default_exotic_input_observes_no_unpriced_bet_type` で固定した。
- **新テーブルの migration が要る。** 共有 DB へ `paddock-analyze migrate` で明示適用する
  （起動時 auto-migrate されない・ADR 0070）。
- `find_race_odds_morning` / `rest-api-read.md` の「朝時点」定義は**挙動不変**（決定 7）。
  既存テスト `morning_returns_earliest_complete_snapshot_with_bounds` /
  `incomplete_snapshot_converges_to_complete_via_upsert` が緑のままであることで担保する。

## 関連

- [ADR 0010](0010-persist-and-reference-odds.md)（read-through / #294 の cache-hit 規則。本 ADR が部分 supersede）
- [ADR 0086](0086-netkeiba-unpriced-sentinel-is-not-odds.md)（番兵はオッズではない。本 ADR の起点となる副作用を予告）
- [ADR 0088](0088-bet-type-scoped-unpriced-sentinels.md)（券種スコープ判定。本 ADR はその API の上に乗る）
- [ADR 0049](0049-netkeiba-odds-transient-retry-and-degraded-exit.md)（netkeiba オッズ経路にレート制御が無いことの裏書き）
- [ADR 0068](0068-race-result-ingestion-ui-reflection.md)（netkeiba への無駄打ちを構造的に止める規律・IP ブロック）
- [ADR 0070](0070-explicit-migration-no-auto-on-startup.md)（migration の明示適用）
- #633（未発売の記録方針の一貫性）
