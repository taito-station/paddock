---
# knowledge 規約に基づくメタデータ（docs/knowledge/README.md）。specifications はその場で
# knowledge に昇格（ADR 履歴・相互リンクを壊さないため物理移動しない）。
status: Confirmed
kind: knowledge
doc_class: [D10, D08, D09]
tags: [D10, D08, D09]
updated: "2026-08-22"
---

# netkeiba 当日データソース取り込み 仕様書

Issue #28 対応。当日（これから走る）レースの **出馬表と単勝オッズ・人気** を netkeiba から取得し、
`race_cards` / `horse_entries` / `race_odds` に取り込む。

## 概要

![netkeiba 当日データソース取り込みフロー](diagrams/netkeiba-datasource-dataflow.svg)

paddock のデータ取得は現状 JRA 公式 PDF が前提で、当日入力を自動で揃えられない。

- `parse-pdf fetch` は**結果(seiseki) PDF 専用**で、これから走るレースの出馬表は取得できない。
- 出馬表は `parse-entries` で扱えるが**自動取得の口が無く**、JRA は出馬表を予測可能な固定 URL で配信していない。
- **オッズの永続化(`race_odds`)が未実装**で、`predict` のライブセッションは買い目推奨(EV・Kelly)を出せずスキップになる(#25)。

netkeiba の出馬表ページ(`race/shutuba.html`、EUC-JP)は **出走馬・単勝オッズ・人気が 1 ページ**に揃っており、
`predict` が必要とする `race_cards` と `race_odds` を一括で満たせる。本仕様はこれを当日データソースとして取り込む
新規アプリ `paddock-fetch-card` と、`race_odds` 永続化基盤を定義する。

レイヤー方針: 取得は **新しい取得アダプタ(interface 層)** として追加し、**ドメイン(`RaceCard` / `RaceOdds`)は変更しない**。

---

## スコープ

### 本仕様で実装する

| 項目 | 内容 |
|------|------|
| 出馬表取得 | netkeiba race_id を指定し、枠番・馬番・馬名・騎手を取得して `race_cards` / `horse_entries` に取り込む |
| 単勝オッズ・人気取得 | netkeiba のオッズ API(`type=1`)から単勝オッズ・人気を取得し `race_odds`(単勝)へ保存 |
| race_id 構築 | 構成要素(年/場/回/日/R)からの race_id 構築を支援 |
| 文字コード | 出馬表は EUC-JP → UTF-8 変換を内部で吸収。オッズ API は UTF-8 JSON |
| 取得済み管理 | 出馬表は `fetch_history` 相当で二重取得を抑止 |
| race_odds 永続化 | 汎用スキーマ(`race_odds` テーブル + `save_race_odds`)を新設 |

### スコープ外（別 Issue）

| 項目 | Issue |
|------|-------|
| 組合せ券種オッズ(馬連・ワイド・3連複等)の取得と永続化 | #38 |
| 確定結果の自動取得・予想セッションの自動精算 | #40 |
| 斤量・性齢など未活用特徴量の予想活用（ドメイン拡張を伴う） | #31 |

> 斤量・性齢は shutuba ページに存在するが、現行ドメイン `HorseEntry`(枠番/馬番/馬名/騎手)には対応フィールドが無い。
> 本仕様では**ドメインを変更しない**方針に従い、取得しても破棄する（保存しない）。活用は #31 でドメイン拡張とあわせて行う。

---

## 取得対象ページと項目

### 出馬表ページ

```
https://race.netkeiba.com/race/shutuba.html?race_id=<netkeiba_race_id>
```

出馬表ページの**静的 HTML には単勝オッズ・人気が含まれない**（`---.-` / `**` のプレースホルダで、JS が別 API から描画する）。したがって出馬表(カード)とオッズは別経路で取得する **2-fetch 構成**とする。

| 取得項目 | 用途 | 保存先 | 取得元 |
|---------|------|-------|-------|
| 枠番 | `HorseEntry.gate_num` | horse_entries | 出馬表 HTML(`td.Waku`) |
| 馬番 | `HorseEntry.horse_num` | horse_entries | 出馬表 HTML(`td.Umaban`) |
| 馬名 | `HorseEntry.horse_name` | horse_entries | 出馬表 HTML(`td.HorseInfo a`) |
| 騎手 | `HorseEntry.jockey` | horse_entries | 出馬表 HTML(`td.Jockey a`) |
| 芝/ダ・距離 | `RaceCard.surface` / `distance` | race_cards | 出馬表 HTML(`div.RaceData01`) |
| 開催日 | `RaceCard.date` | race_cards | 出馬表 HTML(`YYYY年M月D日`) |
| レース名 | `RaceCard.race_name`（#389・best-effort） | race_cards | 出馬表 HTML(`h1.RaceName`。グレードは含まない) |
| 格付け | `RaceCard.race_class`（#345・best-effort） | race_cards | 出馬表 HTML(`<title>`グレード＋`div.RaceData02`条件) |
| (斤量・性齢) | — | 破棄(スコープ外) | — |

`RaceCard.venue` / `round` / `day` / `race_num` は race_id（12桁）から導出する。

### 単勝オッズ API

```
https://race.netkeiba.com/api/api_get_jra_odds.html?race_id=<netkeiba_race_id>&type=1&action=update
```

UTF-8 の JSON を返す。`data.odds["1"]` が単勝で、キー=馬番2桁ゼロ詰め、値=`[オッズ, "0.0", 人気]`。

| 取得項目 | 用途 | 保存先 |
|---------|------|-------|
| 単勝オッズ | `race_odds.odds`(bet_type=win) | race_odds |
| 人気 | `race_odds.popularity` | race_odds |

レース前で未確定(`---.-`)の馬は除外する。単勝・複勝は `type=1`、組合せ券種（馬連・ワイド・馬単・
三連複・三連単）は `type=4/5/6/7/8` で取得する（#102 / #187）。fetch-card の保存も、predict /
predict-watch / api-server の live scrape（`OddsInteractor` への `UreqNetkeibaScraper` 注入）も
すべてこの netkeiba オッズ API（UTF-8 JSON）に統一されている（#287 / ADR 0048。旧 JRA odds-scraper は撤去）。

---

## race_id 構築規則

netkeiba race_id は 12 桁: **`YYYY` + 競馬場2桁 + 開催回2桁 + 日次2桁 + レース番号2桁**。

例: `202605030211` = 2026年・東京(05)・3回・2日・11R(安田記念)

### 競馬場コード

| コード | 競馬場 | コード | 競馬場 |
|--------|--------|--------|--------|
| 01 | 札幌 | 06 | 中山 |
| 02 | 函館 | 07 | 中京 |
| 03 | 福島 | 08 | 京都 |
| 04 | 新潟 | 09 | 阪神 |
| 05 | 東京 | 10 | 小倉 |

### 入力方法

- **直接指定**: 12 桁 race_id をそのまま渡す。
- **構成要素指定**: 年・競馬場(日本語名 or slug)・回・日・R を渡し、ヘルパで 12 桁を組み立てる。

paddock 内部の `RaceId` も同じ 12 桁構成要素から導出する（既存の race_card / race_odds と突合できるキーにする）。

---

## race_odds 永続化スキーマ

券種を限定しない**汎用 1 テーブル**として設計する。#28 では単勝(win)のみ populate していたが、
#38 で組合せ券種(馬連・ワイド・馬単・3連複・3連単)を**マイグレーション再設計なし**で追加した
（`bet_type`/`combination_key` 汎用列にそのまま載る）。

### `race_odds` テーブル

| カラム | 型 | 説明 |
|--------|----|----|
| `race_id` | TEXT | レース識別子(paddock RaceId) |
| `bet_type` | TEXT | 券種(`win` / `place` / `quinella` / `wide` / `exacta` / `trio` / `trifecta`) |
| `combination_key` | TEXT | 組合せキー。単勝は馬番。組合せ券種は昇順連結(例 `03-07`) |
| `odds` | REAL | オッズ(ワイド/複勝は下限) |
| `odds_high` | REAL NULL | オッズ上限(ワイド/複勝のバンド用、単勝は NULL) |
| `popularity` | INTEGER NULL | 人気(取得できた場合) |
| `fetched_at` | TEXT | 取得時刻(ISO8601) |

- 主キー: `(race_id, bet_type, combination_key)`
- オッズは時々刻々変動するため、**取得のたびに最新値で upsert(上書き)** する。履歴保持はスコープ外。
- ドメイン `RaceOdds.win`(`HashMap<HorseNum, OddsValue>`)は人気を持たないため、人気は本テーブルのカラムとして
  scrape 結果から直接保存する（ドメイン型は変更しない）。

### 未発売の番兵値（#621・ADR 0086、券種スコープ化 #630/#634・ADR 0088）

netkeiba は**未発売・該当なしの組み合わせ**に固定の番兵値を入れる。**払戻倍率ではない**ので
オッズとして採用しない。**判定は券種スコープ**（ADR 0088。番兵は「その券種に netkeiba が入れる
固定値」であり、同じ値でも券種が違えば正当な配当——ワイドの `9999.9` は番兵、三連複の `9999.9` は
正当。9000〜11000 帯に trio 6,244 行・trifecta 56,230 行の正当配当が実在=2026-08-18 実測）。

| 券種 | 番兵値 | 備考（DB 実測） |
|---|---|---|
| 単勝 / 複勝 | **（番兵なし）** | 番兵値の行は 0 行（#634・2026-08-18）。未発売の馬は行ごと欠ける形で観測される |
| ワイド | `9999.9` | 相方が `odds_high=0.0` になるため、従来は**下限違反として偶然弾かれていた**（DB には 0 行） |
| 馬連 / 馬単 / 三連複 | `99999.9` | **ここが素通りして EV を壊していた**（#621 の実害。`race_odds` に trio 1,599 行 / exacta 859 行 / quinella 156 行） |
| 三連単 | `999999.9` | 33,176 行（2026-08-18）。**DB に残っていた番兵値 2 種のうち、issue が挙げていなかった方** |

- **判定は券種ごとの特定値の除外**（epsilon `1e-6` 比較）。**上限方式は採らない**——三連単には
  `111971.9` / `200886.6` のような正当な高配当が実在し、上限は大穴を殺す。ADR 0086 が許容していた
  誤爆（他券種の番兵値と同値の正当配当を落とす）は ADR 0088 で撤回した。
- 判定は `OddsValue::try_from((BetType, f64))`（`src/domain/src/odds/odds_value.rs`）の 1 か所。
  **券種は必須入力**で、`TryFrom<f64>` は存在しない——券種を渡し忘れた新しい呼び出し口は
  コンパイルエラーになる（ADR 0088）。`save_race_odds` と `find_race_odds` が委譲しているので
  **保存・読み出しの双方に効き、既に DB にある番兵行も読み出し時に券種別に無害化**される
  （既存行の DELETE は不要）。保存側は未知 `bet_type` ラベルの行を warn+skip する
  （券種を解決できない行は番兵ガードを通せないため書かない）。
- 番兵は `Error::UnpricedSentinel` として値域違反（`OutOfRange`）と区別し、**ログは `debug`**。
  「まだ売れていない」という正常な状態で 1 レースに数百件出るため、warn にすると本来の値域違反が
  埋もれる。
- **例外: ワイドの未発売行は保存時 `warn` のまま**。上表のとおり相方が `odds_high=0.0` になり、
  `save_race_odds::classify_row` は 1 行に値域違反が混ざれば warn 側を優先する（番兵に引っ張られて
  debug に落とすと本来見るべき残骸が埋もれるため）。ある値が番兵か否かは券種別（ADR 0088）だが、
  **warn / debug の分岐そのものは券種ではなく成分の内訳**で決まる。弾かれた成分が全部番兵なら band でも `debug` になる
  （`[9999.9, 9999.9]` の形。現行 netkeiba は返さないが契約として単体テストで固定）。読み出し側は
  成分ごとに判定するのでワイドの番兵も `debug`。「番兵起因の WARN は 0 行」という #621 の実測は
  `--overview`＝**読み出し経路**での計測であり、保存経路のワイドを含意しない。
- **記録方針は「未発売は記録に値する」で一貫・表現は 2 層**（#633・ADR 0090）。
  **過去分**＝`race_odds_snapshots` に残る番兵の生値は「その時点で未発売だった」歴史的事実として
  **保持する**（DELETE しない・ADR 0086 決定 3。読み出しは券種スコープ判定で無害化済み）。
  **今後分**＝未発売は番兵でなく観測表 `race_odds_unpriced_observations`（ADR 0089・現在状態の
  マーカー。載るのはスクレイプ経路で未発売と確認できた券種のみで fetch-card 自身と全券種
  未公開の時間帯は記録されない＝ADR 0089「影響」）が持ち、**snapshots へ番兵や未発売フラグ行を
  積み直すことはしない**（オッズ＝EV 用データと運用観測を同じ表に混ぜない）。
  「いつ発売されたか」の時系列が要件になったら観測表の append-only 化を別 ADR で判断する
  （#649 の実測が材料）。

#### 番兵リストの正本ファイル

正本は **`src/domain/src/odds/netkeiba_sentinels.txt`**（TAB 区切り `券種<TAB>値` の 2 列。
券種ラベルは Rust `BetType` の snake_case。**番兵を持たない券種＝win / place は行そのものを
置かない**。コメント行は書けない——両言語のパーサ規則を「空行スキップ + split」に保つ・ADR 0088）。
Rust と Python が**同じファイルを読む**。同じ値を両言語が別々に持つと片方だけ更新して静かに
ズレるため（#587 の見出し契約と同型の事故）、言語をまたぐ golden で結ぶ（ADR 0085 の前例）。

| 読む側 | 読み方 | ファイルが壊れると |
|---|---|---|
| Rust `src/domain/src/odds/odds_value.rs` | `const NETKEIBA_SENTINELS` を持ち、テスト `sentinel_list_matches_the_shared_golden` が `include_str!` で突き合わせる（`#[cfg(test)]` 内） | **テストビルド**が落ちる（本番ビルドは通る） |
| Python `scripts/predict-check/odds_guard.py` | **import 時**に読んで集合を作る | import した解析スクリプトが**起動時に**落ちる（**必ずパスを示し**、行に起因するものは行番号と該当行も出して停止する。空リストへのフォールバックはしない——番兵が素通りするため） |

Python 側は**欠落 / 列数不正（2 列でない行）/ 未知の券種ラベル / 非数値の値 / 非 UTF-8 保存 /
非有限値（`nan`・`inf`）/ 同一（券種, 値）の重複行 / 空**をすべて拒否する。重複判定は実行時の
番兵一致と同じ epsilon 比較（近接値のコピペ事故も同一扱い）。非有限を受理しないのは、番兵として登録しても
`abs(o - nan) < ε` が常に偽になり**その値だけが黙って無効化**されるため（空を拒否するのと同じ理由）。
Rust 側は golden を const と完全一致（券種・値・順序）で突き合わせるので、同じ壊れ方は
`sentinel_list_matches_the_shared_golden` が落とす。

**`testdata/` に置かない。** Python が import 時に読む**本番依存**であり、テスト専用資産ではない。

**番兵値を足すときは 3 か所を同じ PR で更新する**: この正本ファイル / Rust の `NETKEIBA_SENTINELS` /
`scripts/predict-check/test_odds_guard.py` の期待 dict。どれか 1 つを忘れれば Rust か Python の
テストが落ちる。win / place に番兵を足す（＝行を置く）ときは、Rust の
`win_and_place_have_no_sentinels` が落ちるので #634 の実測を覆す根拠を PR に示す。

`scripts/` が Rust のガードを通らないのは、psql / TSV で DB を直読みするため（値オブジェクトを
一切経由しない）。Python の公開 API も券種必須（`is_sentinel(bet_type, odds)` /
`is_payout_odds(bet_type, odds)`・未知ラベルは `ValueError`）で、既定値による更新漏れの素通りを
塞ぐ（ADR 0088）。

### 保存したオッズの読み出しと read-through（ADR 0010）

書き込み（fetch-card → `race_odds`）だけがあって読み出しが無い状態を解消した決定。

- **`Repository::find_race_odds(race_id, as_of)`**: `race_odds` をドメイン `RaceOdds` に再構成する。
  `as_of = Some(d)` は `date(fetched_at) <= d` のスナップショットに限定し（**backtest のリーク防止**）、
  `None` は時刻制約なし（predict）。
- **predict は read-through**: 保存済みがあればそれを返し、無ければライブスクレイプして保存してから返す。
- **backtest は当時オッズ優先・PDF フォールバック**: `find_race_odds(race_id, Some(race.date))` の win が
  あればそれ、無ければ PDF 確定成績の単勝を使う。保存オッズが無い過去レースでも既存の長期バックテストが
  壊れない（移行コストゼロ）。
- **cache-hit 判定の基礎は `RaceOdds::is_complete()`**（win + 組合せ 5 券種がそろう）。当初の「保存済みが
  空でない」判定では、組合せ券種の一部が欠けた**部分スナップショット**が cache-hit してしまい、欠落券種が
  当日ずっと取り直されなかった（#294 で強化）。`race_odds` は単一行 UPSERT なので、再スクレイプは欠けていた
  行を足すだけで既存行を消さない＝保存済み券種は単調に埋まり complete に収束する（自己修復）。
- **`place` は cache-hit 条件に含めない**。netkeiba は win と同梱で複勝を返すため通常そろうが、発走前の
  複勝未公開で再スクレイプが無限化するのを避ける。
- **欠落のうち「未発売と確認できた券種」は差し引く（ADR 0089・#632）**。`is_complete()` だけを見ると、
  券種がまるごと未発売の時間帯（前日プリフェッチ）は当該券種が永久に 0 行になり
  （番兵は `race_odds` に入らない・ADR 0086）、read-through を呼ぶたびに 6 GET のフルスクレイプが走る。
  netkeiba 経路には RateGate が無く（ADR 0049）IP ブロックが最重要運用リスク（ADR 0068）なので、
  構造で止める。
  - **cache-hit の式**: 保存済みの欠落券種（`RaceOdds::missing_bet_types()`）が、**TTL 15 分以内の
    未発売観測にすべて収まる**なら再スクレイプしない。
  - **未発売の判定は「取得に成功したのに priced が 0 件か」**。「全行が番兵か」では判定しない
    ——実地のワイド未発売行は `["9999.9", "0.0", "--"]` で相方 `0.0` が値域違反として弾かれるため、
    番兵の有無だけを見ると取りこぼす。JRA がそもそも売らない極小頭数レースの券種（0 行）も同じ経路で拾う。
  - **観測できた券種だけが対象**。スクレイパは取得に成功した券種を `FetchedExoticOdds::observed` で
    伝え、そこに無い券種（取得失敗・そもそも取りに行っていない）はマークしない＝次回そのまま
    取り直す＝**#294 の自己修復は鈍らない**。「失敗集合」でなく「観測集合」を持つのは、
    `Default`（空）が最も安全な解釈「何も観測していない」になるようにするため。
  - **安全側の絞り込みが 2 つ**: 単勝の観測は cache-hit 判定で無視する（win 空のまま
    cache-hit するのを防ぐ。複勝は `missing_bet_types()` が返さないので除外不要）／
    `observed_at` が未来の観測は stale 扱いにする（時計ズレで再取得が止まらないように）。
  - **効くのは「単勝は取れるが組合せ券種が未発売」の状態**。全券種が空のレース（単勝すら未公開）と
    `fetch-card` 自身は観測を残さないので、そこは従来どおり取り直す（ADR 0089「影響」）。
  - 観測は専用表 `race_odds_unpriced_observations(race_id, bet_type, observed_at)` に置く
    （番兵はオッズではないので `race_odds` に入れない・ADR 0086 決定 1/3）。priced が取れた券種の
    マークは同一トランザクションで削除する。**保存（`save_race_odds`）に失敗した回は観測を
    記録しない**——古いスナップショットに新しいマークが付くと TTL のあいだ古い値を返し続ける。
  - **記録するのは read-through（`race_odds`）と `predict-watch`（`refresh_race_odds`）の 2 経路**。
    監視側は cache を見ないが同じ観測を残す。5 分毎に回る監視が**発売開始を最初に観測して
    マークを消す**のが通常で、これにより read-through 側も次回すぐ取り直せる。
    `fetch-card` は `assemble_netkeiba` を通らないので記録しない。
  - **TTL 15 分＝発売開始に気づくまでの最大遅れ**。発走直前の鮮度は `predict-watch`
    （read-through を通らず毎回再取得・#257）が担保するので判断には影響しない。
  - **`is_complete()` の意味は変えていない**（priced が全券種そろっているか）。したがって
    `find_race_odds_morning` の「朝時点」定義は挙動不変（ADR 0088 / `rest-api-read.md`）。
  - 単複のみの取得（odds-collect）は組合せ券種を観測しないので未発売マークを作らない。
- 当初は win+place 限定だったが、**#38 で全券種**（馬連・ワイド・馬単・3連複・3連単）に拡張済み。
  `combination_key` の規約はドメイン型の `to_key`/`from_key` が単一情報源（昇順 `-` 連結、順序付きは `>` 連結）。
  `BetType` で解釈できない未知ラベルの行は読み飛ばす（新版が書いた券種を旧版で読む過渡期でも止めない）。
- **時刻比較の粗さ**: 当時オッズ参照は `date(fetched_at)`(UTC) と `race.date`(JST 開催日) の日付比較。
  TZ 境界は厳密でないが、fetch-card / predict をレース前に走らせる運用前提で実害は小さい。

### transient リトライと degraded（ADR 0049）

「取れなかった」を**未発売**と**一過性障害**に機械的に分ける。混同すると、本来取れるはずのオッズの
欠落がサイレントに握り潰される。

- **共有 GET ヘルパ `call_with_retry`** が transient 失敗時に最大 3 回（初回 + 2 回）・指数バックオフ
  （1s / 2s）で再試行する。transient は `Timeout` / `Io` / `ConnectionFailed` / `HostNotFound` /
  `Protocol` / 5xx。リトライは I/O 層の性質として `fetch_utf8` / `fetch_decoded` の双方に効かせる
  （オッズ以外の出馬表・近走・払戻も同時に resilience が上がる）。
- **netkeiba に 403/404=absent の概念は無い**。未発売は 200 + JSON status で返るため、4xx は単純に
  非 transient として扱う。
- **degraded になるのは単複オッズ取得の失敗だけ**。それ以外はハード失敗か best-effort に分かれる:
  - **card 取得段の出馬表**（`fetch_card`）の失敗 → ハード失敗（exit 1）
  - **組合せ券種（exotic）オッズ**の失敗 → **警告のみ・単複だけ保存して exit 0**。部分スナップショットは
    cache-hit 判定 `is_complete()` が false のままなので、次回 read-through で欠けた券種が埋まる（自己修復）。
    **失敗した券種に未発売マークは付かない**ので、ADR 0089 の TTL でこの自己修復が遅れることはない
  - **近走取り込み段**（`horse_history`）で引く出馬表・馬ページの失敗 → **警告のみ・exit 0**
    （`shutuba_failed` / `horses_failed` に計上）。card / オッズ保存まで成功した実行を近走の失敗で
    巻き添えにしない。**card 取得済みで再実行すると必ずこの経路を通る**ので、近走が 1 件も
    取れなくても終了コードは 0 になる——件数はログで見る
- **未発売は best-effort**（出馬表・近走を巻き添えにせず継続）、**transient は degraded**。degraded では
  **オッズ保存をまるごとスキップする**——win 欠落の部分スナップショットを永続化すると predict が
  「オッズ有り・win 無し」で誤判定するため。保存しない方が「オッズ未取得」として扱われ、再取得で正される。
- **degraded は専用 exit code = 3**。`fetch-card` は主目的（近走取り込み）まで終えてから 3 を返すので、
  呼び出し側はハード失敗（=1）と「単複だけ未取得・要再取得」を区別でき、win 欠落レースだけ再取得できる。

### 取り込み対象外レース（障害）のスキップ（ADR 0075）

「対応外だからスキップした」と「netkeiba 側で失敗した」を、**終了コードと stdout で区別できる**形にする。
混同すると、開催日の全レースをループ取得したとき 1 件の欠落が無視してよいものか取り込み失敗かを
件数 diff でしか判別できない。

- **障害レースは取り込み対象外**。判定は `RaceData01` の**距離付き馬場マーカー**で行い、`障3000m`・
  `障芝3000m`・`障ダ2900m` のいずれの表記でも拾う（単文字クラスだと `障芝3000m` で `芝3000m` に
  マッチし、障害レースが芝レースとして黙って取り込まれる）。`parse_card` は `Error::Unsupported` を返し、
  実障害（`Error::Parse` → `Internal` → exit 1）とは別 variant で ingest / CLI まで伝わる。
- **ingest はカード・オッズ・近走のすべてを打ち切る**（DB は一切変更しない）。カード無しでオッズだけ
  保存すると `race_cards` に対応行の無い孤児オッズが残り、近走取り込みは障害レースでも成功して
  しまうため。
- **CLI は理由を stdout に明示して exit 0**。専用 exit code は作らない——障害レースが実際に到達する
  消費側は「netkeiba の開催一覧からレースを列挙して回すループ」（`scripts/predict-check/README.md` の
  手順）で、exit code だけでレース単位の成否を判断するため、専用コードでは対応外レースが取り込み失敗に
  計上されてしまう。
- 馬場・距離表記が読めない場合や `RaceData01` 欠落は従来どおり実障害（exit 1）。**「対応外」は広げない**。
- **スキップの識別には stdout の読み取りが要る**（exit code だけでは正常取り込みと区別できない）。
  当該**行の行頭**が `スキップ: ` 固定（tracing のログ行が同じ stdout に混ざるため行単位で照合する）。
  **stdout を捨てる消費側（`refresh_ev.sh` は `> /dev/null`）からは追えない**——
  stderr には出さない（同スクリプトが「stderr あり」を異常として警告するため、正常な結果を警告に
  化けさせない）。tracing も既定 writer が stdout なので代替にならない。
- 却下案（ADR 0075 に詳細）: 専用 exit code の新設 / `IngestCardResponse` にフラグを足す（degraded と同型）/
  エラー文言の照合 / 障害レースを `Surface::Jump` として取り込む——いずれも採らない。

### スクレイパー実装の型（ADR 0001 由来）

JRA 版スクレイパーは ADR 0048 で退役したが、そこで確立した設計は netkeiba 版にも引き継いでいる。

- **HTTP クライアントは同期 `ureq` に統一**する（async ランタイムと 2 系統の HTTP スタックを混在させない）。
- **レイヤー配置は依存方向 Apps → Interface → Use-Case → Domain を厳守**する。ドメイン型
  （`BetType` / `OddsValue` / `PlaceOdds` / `Pair` / `OrderedPair` / `Triple` / `OrderedTriple` / `RaceOdds`）は
  domain、port トレイトは use-case、HTML/JSON パースと遷移層は interface の専用クレート。
- **検証は保存 fixture に対する純粋関数で行う**。ライブの遷移はサイト改変で壊れやすく開催日にしか
  実地検証できないため、価値の中心であるパース／ドメイン変換を純粋関数として切り出して決定論的に検証する。

---

## 取得済み管理（dedup）

- **出馬表(静的)**: `fetch_history` 相当に「この race_id の出馬表を取得済み」を記録し、再実行時はスキップする。
  `--force` で再取得を強制できる。
- **オッズ(変動)**: dedup しない。実行のたびに取得し、`race_odds` を最新値で upsert する。

> 出馬表をスキップした場合でも、オッズ取得・更新は継続する（フロー図の右側分岐）。

---

## エラー・欠損ハンドリング

| ケース | 挙動 |
|--------|------|
| 単勝オッズ未確定(レース前で空欄) | win を populate せず空のまま。出馬表の保存は継続 |
| 不正な race_id(桁数・競馬場コード不正) | バリデーションエラーを stderr に出力し exit code 1。パニックしない |
| 障害レース(取り込み対象外) | 取り込みを行わず理由を **stdout** に出して **exit 0**。DB は無変更。ハード失敗(1)・degraded(3) と区別する。ADR 0075 |
| **単複オッズ**の取得失敗（transient・リトライ後も残る） | **degraded**。オッズ保存をまるごとスキップし exit 3（出馬表・近走は保存済み）。ADR 0049 |
| **単複オッズ**の取得失敗（4xx 等の非 transient） | 同じく degraded 分岐（オッズ未保存・exit 3）。netkeiba に 403/404=absent の概念が無いため 4xx も「取れなかった」として扱う |
| **組合せ券種オッズ**の取得失敗 | warn して単複のみ保存・**exit 0**。`is_complete()` が false なので次回再取得で埋まる（失敗券種に未発売マークは付かないので TTL で遅れない・ADR 0089） |
| **組合せ券種オッズ**が未発売（番兵のみ / 0 行） | fetch-card は通常どおり単複のみ保存して **exit 0**（未発売は best-effort）。**未発売の観測記録は fetch-card では行わない**——`assemble_netkeiba` を通らないため。記録するのは read-through / `predict-watch` 経路（下記 read-through 節・ADR 0089） |
| **出馬表**ページの取得失敗（card 取得段） | `call_with_retry` のリトライ後も残ればハード失敗（exit 1）。degraded にはしない |
| **近走取り込み段**の取得失敗（出馬表・馬ページとも） | warn + skip して `shutuba_failed` / `horses_failed` に計上。**exit 0 のまま**（best-effort）。前走フォーム特徴量が欠けるので、ログの失敗件数を見る |
| EUC-JP 不正バイト | `encoding_rs` の置換に委ね、可能な範囲で parse を継続 |

---

## CLI

```
paddock-fetch-card <netkeiba_race_id>
paddock-fetch-card --year 2026 --venue 東京 --round 3 --day 2 --race 11
```

オプション(予定):

| オプション | 説明 |
|-----------|------|
| `--year / --venue / --round / --day / --race` | 構成要素から race_id を組み立てる |
| `--force` | 出馬表が取得済みでも再取得する |
| `--interval` | netkeiba への連続リクエスト間隔(秒、既定値あり) |

スクレイピング対象は公開ページ。既定ウェイトを入れ netkeiba 側へ配慮する。

### 終了コード

呼び出し側——netkeiba の開催一覧からレースを列挙して回すループ（`scripts/predict-check/README.md`
の手順）——が「本物の失敗」だけを FAIL として扱えるようにするための一次情報。

| コード | 意味 |
|--------|------|
| 0 | 正常終了（**取り込み対象外レース（障害）のスキップを含む**。ADR 0075） |
| 1 | ハード失敗（不正な race_id・ページ取得失敗・パース失敗・DB エラー） |
| 2 | 引数の形式不正（`clap` が stderr にエラーを出力。アプリのコードは関与しない） |
| 3 | degraded（単複オッズ未取得。card は保存済みで要再取得。ADR 0049） |

- **exit 0 は正常終了（対象外スキップを含む）**。対応外レースは異常ではないため exit 0 とし、理由は
  **stdout** に出力する（`predict` の「開催なし日付」と同じ規約。[predict-session.md](predict-session.md) 参照）。
  **取り込み失敗として数えてよいのは 1 だけ**——3 は card 保存済みでオッズのみ要再取得なので、
  FAIL に丸めると「win だけ再取得すればよい」判断を落とす。
- ただし **exit 0 だけでは「取り込んだ」と「対象外でスキップした」を区別できない**。区別が要る消費側は
  stdout を**行単位**で見て `スキップ: ` で始まる行を探す（stdout には tracing のログ行が先行しうるため、
  出力全体の先頭で照合しない）。

---

## 留意（既知の前提）

- **騎手名は netkeiba の略称表記**（例「戸崎圭」）で保存する。JRA PDF 由来の正式名（「戸崎圭太」）とは
  表記が異なるため、クロスソースで騎手成績(`jockey_stats`)を突き合わせる際は取りこぼし得る。
  netkeiba 内では整合する。正規化が必要になれば別 Issue で扱う。
- `RaceCard.venue`/`round`/`day`/`race_num` は race_id 由来、`surface`/`distance`/`date` は HTML 由来で、
  両者の整合チェックは行わない（誤った race_id を渡した場合の検出は本仕様のスコープ外）。

---

## 関連

- ADR: 0008 netkeiba を当日データソースに採用 /
  0001 JRA オッズスクレイパー（設計の型・0048 で退役）/
  0010 オッズの永続化と参照 /
  0048 JRA スクレイパー退役 /
  0049 transient リトライと degraded exit /
  0075 対応外レースは exit 0 + stdout 明示でスキップ
- 関連 Issue: #25(オッズ→predict 結線)、#38(組合せ券種オッズ)、#40(結果自動精算)、#31(未活用特徴量)、#586(対応外レースのスキップ)
- CLI テストケース: `tests/cli-test-cases/fetch-card-command.md`

---

## 決定ログ

<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->

### ADR 0001: JRA オッズスクレイパーの実装 (Issue #10) (2026-06-02) — 承認済み

#### ステータス

承認済み（**ライブ遷移層 `UreqOddsScraper` / `odds-scraper` crate は #287 / ADR 0048 で撤去・supersede**。
live odds 取得は netkeiba の `OddsScraper` 実装に統一。本 ADR のパース設計記録は歴史的価値で残置）

#### コンテキスト
issue #10 で、JRA 公式サイトから当日の馬券オッズ（単勝/複勝/馬連/馬単/三連複/三連単）を
取得する interface クレートが求められた。

調査の結果、JRA のオッズページには以下の本質的制約があることが判明した。

- オッズ画面は race_id を含む安定した GET URL では取得できない。結果 PDF
  （既存 `pdf-parser`）のような直接 URL が存在しない。
- `accessO.html` は GET だとエラーページへ 301 リダイレクトする。
- オッズ画面への遷移は、メニューページの JavaScript リンク
  `doAction('/JRADB/accessO.html', '<cname>')` が持つ **`cname` セッショントークン**を
  `accessO.html` へ POST することで初めて辿れる。
- 開催日（週末・祝日）以外はライブのオッズページ自体が存在しない。

このため「ライブ遷移層」はテストが困難かつ本質的に不安定であり、検証可能な形で
実装範囲を切り分ける必要があった。

#### 決定
1. **HTTP クライアントは `ureq`（同期）に統一する。** issue 本文では reqwest が
   挙げられていたが、既存プロジェクトは `UreqFetcher` をはじめ全て同期 `ureq` に
   統一されており、port トレイト（`PdfFetcher` 等）も同期である。二系統の HTTP
   スタック・async ランタイム混在を避け、一貫性を優先する。
2. **レイヤー配置**（依存方向 Apps → Interface → Use-Case → Domain を厳守）:
   - Domain: `odds` モジュール。`BetType` / `OddsValue` / `PlaceOdds` /
     組番キー（`Pair` / `OrderedPair` / `Triple` / `OrderedTriple`）と、馬券種ごとの
     オッズマップを束ねるアグリゲート `RaceOdds`。
   - Use-Case: port トレイト `OddsScraper`（`scrape(&RaceId) -> Result<RaceOdds>`）。
   - Interface: 新規クレート `odds-scraper`。HTML パーサ（`scraper` クレート）と
     ライブ遷移層 `UreqOddsScraper`。
3. **検証は保存 HTML fixture に対するパーサ／組み立て（`assemble`）で行う。**
   ライブの POST/cname 遷移は best-effort 実装とし、純粋関数 `assemble` を
   検証済みコアとして切り出す（既存 PDF パーサと同方針）。
4. **既存 `Interactor<R, P, F>` には追加しない。** port は単独トレイトとして公開し、
   将来 interactor/app から消費する。本 issue ではアプリ配線・DB 永続化はスコープ外。

#### 理由
- JRA の POST/cname 遷移はサイト改変で壊れやすく、開催日限定でしか実地検証できない。
  価値の中心であるパース／ドメイン変換ロジックを純粋関数として切り出すことで、
  fixture により決定論的に検証できる。
- ureq 統一は in-house の一貫性方針に沿い、依存とランタイムの単純化に寄与する。
- `Interactor` への追加は全 app に DI 強制が波及するため、未配線の段階では避ける。

> **追記（Issue #25 / ADR 0005）**: 本 ADR でスコープ外とした「アプリ配線」は #25 で実施した。
> `odds-scraper` は predict から消費されるようになり、下記 members への明示登録の例外は解消した
> （決定 #4「メイン Interactor に追加しない」は専用 `OddsInteractor` で踏襲）。

#### 影響
- ~~`odds-scraper` は現時点でどのバイナリからも参照されないため、ワークスペースの
  ビルド／テストグラフに含めるべく `Cargo.toml` の `members` に明示的に登録した
  （「members はバイナリのみ」という通常方針に対する明示的な例外）。~~
  → #25 で predict が path 依存で参照するようになり、members 明示登録は撤去した。
- fixture は JRA の公開オッズテーブル構造を代表する形で作成しており、ライブページ
  との完全一致は将来のライブキャプチャで突き合わせる必要がある（精度は暫定）。
  これは既存パーサの既知制約と同様の扱い。
- ライブ遷移層（cname トークン抽出・POST）は実地未検証であり、開催日に実データで
  突き合わせる作業が残る。

### ADR 0008: netkeiba を当日(出馬表・オッズ)データソースに採用 (Issue #28) (2026-06-08) — 承認済み

#### コンテキスト
当日（これから走る）レースの予想に必要な入力（出馬表・単勝オッズ・人気）を、現状の paddock は
自動で揃えられない。

- `parse-pdf fetch` は結果(seiseki) PDF 専用で、これから走るレースの出馬表は取得できない。
- 出馬表は `parse-entries` で扱えるが自動取得の口が無い。**JRA は出馬表を予測可能な固定 URL で
  配信していない**ため、当日分を自動取得できない。
- オッズの永続化(`race_odds`)が未実装で、`predict` のライブセッションは買い目推奨(EV・Kelly)を
  出せずスキップになる(#25)。

メモリ方針では「外部 API より自己完結する解を優先」だが、**当日入力は公式に自動取得する手段が存在しない**。
一方 netkeiba の出馬表ページ(`race/shutuba.html`、EUC-JP)は出走馬・単勝オッズ・人気が 1 ページに揃っており、
`predict` が必要とする `race_cards` と `race_odds` を一括で満たせる。

#### 決定
1. **netkeiba を当日データソースとして採用する。** 公式の自動取得手段が無いため、現実的な代替として
   公開ページをスクレイピングする。既定ウェイトを入れ netkeiba 側へ配慮する。
   出馬表(カード)とオッズは取得元が異なるため **2-fetch 構成**とする: 枠番/馬番/馬名/騎手/芝ダ/距離/開催日は
   出馬表ページの**静的 HTML**から、単勝オッズ・人気は**オッズ API**(`api_get_jra_odds.html?type=1`, UTF-8 JSON)から取得する。
   出馬表 HTML にはオッズのプレースホルダ(`---.-`)しか無く、実値は JS が同 API から描画するためである。
2. **取得は新規アプリ `paddock-fetch-card`(`src/apps/fetch-card`)に集約する。** parse-pdf(fetch/ingest)・
   parse-entries と並ぶ対称構造とし、PDF 専用だった既存アプリの責務を汚さない。
3. **既存資産を再利用・拡張する。** HTTP は `ureq`(ADR 0001 の統一方針)、文字コードは
   `encoding_rs::EUC_JP`。`NetkeibaScraper` port に出馬表フル取得用メソッドを追加し、
   既存 `fetch_shutuba`(近走取得用、`RunnerRef` のみ返す) は壊さない。
4. **`race_odds` 永続化は券種非依存の汎用 1 テーブルで新設する。**
   `(race_id, bet_type, combination_key, odds, odds_high, popularity, fetched_at)`。#28 では単勝(win)のみ
   populate し、#38 の組合せ券種をマイグレーション再設計なしで受けられるようにする。オッズは変動するため
   常に最新値で upsert する。
5. **ドメイン(`RaceCard` / `RaceOdds`)は変更しない。** 出馬表の斤量・性齢は取得しても破棄する。
   人気は `race_odds` テーブルのカラムとして scrape 結果から直接保存する(ドメイン型は触らない)。

#### 理由
- 当日入力に公式自動取得手段が無い以上、自己完結方針は「公式手段の範囲で完結」では達成できない。
  netkeiba 採用はこの制約下での現実的な選択であり、その位置づけを ADR に明記して方針との整合をとる。
- 新規アプリへの集約は、parse-pdf/parse-entries と同じ「1 アプリ 1 取得経路」のパターンを踏襲し、
  責務分離と将来拡張(#38/#40)の足場になる。
- 汎用 race_odds 表は、単勝のみの最小実装に比べ初期コストは僅かに増えるが、#38 でのスキーマ再設計・
  データ移行を回避できる。券種をキー(`bet_type` + `combination_key`)で正規化することで一様に扱える。
- ドメイン不変は、出馬表 PDF 取り込みや predict・backtest が依存する既存ドメインへの波及を避けるため。
  斤量・性齢の活用は #31 でドメイン拡張とあわせて設計する。

#### 影響
- `race_odds` マイグレーション・`Repository::save_race_odds`・`NetkeibaScraper` の新メソッド・新規アプリ
  `fetch-card` を追加する。`Cargo.toml` の workspace members にアプリを登録する。
- スクレイピング依存のため、netkeiba 側の HTML 改変で parser が壊れうる。parse はフィクスチャ(保存 HTML)に
  対するユニットテストで決定論的に検証する(既存 PDF/オッズパーサと同方針)。
- 単勝のみ populate のため、`predict` の組合せ券種 EV は引き続き #38 待ち。本 ADR はその供給基盤を用意する。
- 出馬表 dedup は `fetch_history` を用い、オッズは upsert で最新化する二系統の更新方針となる。

#### 関連
- 仕様書: [netkeiba 当日データソース取り込み](../specifications/netkeiba-datasource.md)
- ADR 0001(JRA オッズスクレイパ)/ ADR 0005(オッズ→predict 結線, #25)
- Issue: #28 / #38 / #40 / #31 / #25

### ADR 0010: オッズを永続化し predict/backtest から参照する (Issue #51) (2026-06-09) — 承認済み

#### コンテキスト
オッズの扱いは段階的に決めてきた:

- **ADR 0001（#10）**: `OddsScraper`（`scrape(&RaceId) -> RaceOdds`、都度スクレイプ・キャッシュなし）を実装。DB 永続化は別 Issue へ先送り。
- **ADR 0005（#25）**: predict にオッズを結線する際、案A（オンデマンド・都度スクレイプ）を採用し、スタブだった `Repository::find_race_odds` と `race_odds` 永続化（案B）を撤去。予想の再現や当時オッズ参照は将来 Issue へ先送りとした。
- **#28（PR #56）**: `race_odds` テーブル（`(race_id, bet_type, combination_key)` 主キー、`odds`/`odds_high`/`popularity`/`fetched_at`）と `save_race_odds`、`fetch-card` 経由の**単勝**取得・永続化を実装。

この結果、書き込み（fetch-card → race_odds）はあるのに読み出しが無く、predict は依然として毎回ライブスクレイプ、backtest は PDF 確定成績の単勝のみを使っており、保存済みオッズが活用されていなかった。本 Issue（#51）はこの読み出し側を仕上げ、ADR 0005 が先送りした「案B（永続化参照）」を **win+place に限って**採用する。

#### 決定

1. **複勝(place)の取得・永続化を fetch-card に追加する。**
   netkeiba のオッズ API（`api_get_jra_odds.html?type=1`）は 1 レスポンスに単勝(`data.odds["1"]`)と複勝(`data.odds["2"]`)を同梱するため、`fetch_win_place_odds` として 1 回の取得で両方を返す。複勝は幅 odds なので `odds`=下限・`odds_high`=上限に保存する。

2. **読み出し `Repository::find_race_odds(race_id, as_of)` を新設する。**
   `race_odds` の `bet_type IN ('win','place')` をドメイン `RaceOdds` に再構成する。`as_of = Some(d)` のとき `date(fetched_at) <= d` のスナップショットに限定（backtest のリーク防止）、`None` で時刻制約なし（predict）。

3. **predict のオッズ取得を read-through に切り替える。**
   `OddsInteractor` に `Repository` を注入し、`race_odds()` を「保存済み(win+place)があれば返す → 無ければライブスクレイプし win+place を保存してフルのオッズを返す」とする。cache-miss 時に取得したフル（exotic 含む）はその回の買い目にそのまま使うが、保存・再参照は win+place に限る。

4. **backtest は当時オッズを優先し、無ければ PDF にフォールバックする。**
   トップ選好馬の回収率に使う単勝オッズを、`find_race_odds(race_id, Some(race.date))` の win があればそれ、無ければ従来どおり PDF 確定成績の `r.odds` を使う。

5. **組合せ券種（馬連・ワイド・3連複・3連単）の永続化はスコープ外**とし #38 に委ねる。`combination_key` 規約と netkeiba 取得は #38 で定義する。

#### 理由
- 書き込みだけ存在して読み出しが無い状態を解消し、#28 で用意した `race_odds` を実際に活用する。予想の再現性（同一セッション再実行・resume で同じオッズ）と、当時オッズに基づく現実的なバックテスト回収率が得られる。
- ADR 0005 が撤去した案B を**全面復活ではなく win+place に限定**することで、未確定な exotic の `combination_key` 規約（#38）に踏み込まずに必要な価値（単複の再現・複勝の期待値計算）を取れる。
- backtest の PDF フォールバックにより、保存オッズが無い過去レースでも既存の長期バックテストが壊れない（移行コストゼロ）。
- read-through 方式は cache-miss 時もフルのオッズで買い目を出せるため、exotic 推奨の回帰を初回実行では起こさない。

#### 影響
- `OddsInteractor<O>` が `OddsInteractor<O, R: Repository>` になり、predict の `setup.rs` で `SqliteRepository`（プール共有）を注入する。ADR 0001/0005 の「OddsInteractor は永続化を持たない」前提は本 ADR で更新される。
- 保存済み win+place を参照する resume・再実行では exotic 推奨が出ない（exotic は #38 で永続化されるまで cache-miss 時のフルスクレイプ回のみ）。
- スクレイプ由来の保存行は人気(`popularity`)を持たない（netkeiba の fetch-card 経由のみ人気が入る）。`popularity` は NULL 許容なので問題ない。
- backtest の回収率は、当時オッズが保存されたレースでは PDF 確定単勝ではなく当時オッズ基準になる（より現実的）。
- read-through の cache-hit 判定は「保存済み win/place が空でない」。単勝のみ保存された回（複勝未公開時など）は cache-hit して複勝を取り直さない。netkeiba/JRA は単複を同一レスポンスで返すため通常は両方そろうが、片側保存のエッジでは複勝が埋まらないことを許容する（必要になれば「両方そろうまで cache-miss 扱い」に強化する）。
- backtest の当時オッズ参照は `date(fetched_at)`(UTC) と `race.date`(JST 開催日) の粗い日付比較。TZ 境界（レース後の深夜取得は取りこぼし、当日内取得は同日付で通過）は厳密でないが、fetch-card/predict をレース前に走らせる運用前提で実害は小さい。

#### 後日談（#38 で更新）
本 ADR の決定 5 と「影響」の win+place 限定は **#38 で解消**した。`OddsInteractor` は
スクレイプで得た**全券種**（馬連・ワイド・馬単・3連複・3連単を含む）を `race_odds` に保存し、
`find_race_odds` も全券種を読み戻すようになった。これにより resume・cache-hit 時も exotic 推奨が
出る。`combination_key` 規約はドメイン型（`Pair`/`OrderedPair`/`Triple`/`OrderedTriple`）の
`to_key`/`from_key` を単一情報源とする（昇順 `-` 連結、順序付きは `>` 連結）。スキーマ（汎用
`bet_type`/`combination_key`）はマイグレーション再設計なしでそのまま受けられた。`find_race_odds` は
SQL の `bet_type` フィルタを撤廃して全行を読むが、`BetType` で解釈できない未知ラベルの行は
読み飛ばす（撤廃前の「未知は無視」挙動を維持し、将来券種を書く新版 → 旧版で読む過渡期でも
predict/backtest を止めない）。なお組合せ券種は 1 レースで行数が増える（三連単は最大
18×17×16 = 4896 通り）が、`find_race_odds` は PK 先頭 `race_id` で 1 レースに絞って読むため
許容範囲とする。

#### 後日談（#294 で更新）
本 ADR「影響」の cache-hit 判定「保存済みが空でない（`!is_empty()`）」は **#294 で強化**した。
`fetch_one_exotic` は各組合せ券種の一過性失敗を空 Vec に畳む（券種ベストエフォート）ため、
**win は成功・組合せ券種の一部が欠落**した部分スナップショットが生まれうる。これが `!is_empty()` を
満たして cache-hit すると、欠落券種（馬連・ワイド・三連複 等）が当日ずっと取り直されない不具合があった。

cache-hit 判定を **`RaceOdds::is_complete()`（win + 組合せ 5 券種＝馬連・ワイド・馬単・三連複・三連単が
そろう）** に変更した。不完全なスナップショットは cache-miss として再スクレイプする。`race_odds` は
PK=(race_id,bet_type,combination_key) の **単一行 UPSERT**（`save_race_odds`。時系列履歴は別テーブル
`race_odds_snapshots` #232）なので、再スクレイプは欠けていた券種の行を追加するだけで既存行を消さない。
よって保存済みの券種集合は取得済み券種の和集合として単調に埋まり、complete に収束する
（`persist_all` 側は変更不要・自己修復）。なお JRA が一部券種を発売しない極小頭数レースでは
is_complete が常に false になり read-through で毎回再スクレイプするが、UPSERT で行は肥大せず呼び出しも
1 レース 1 回程度のため許容する。

`place` は **引き続き cache-hit 条件に含めない**。netkeiba は win と同梱で複勝を返すため通常そろうが、
上記「複勝未公開時は win-only で cache-hit を許容（両方そろうまで cache-miss にはしない）」方針を維持し、
発走前の place 未公開で再スクレイプが無限化するのを避ける。影響は read-through を使う predict /
api-server のみ（predict-watch は `refresh_race_odds` で毎回再スクレイプするため元々無関係）。

#### 関連
- ADR 0001（JRA オッズスクレイパー実装, #10）
- ADR 0005（predict にオッズを結線, #25）— 本 ADR が案B を限定的に復活させる
- Issue #28（race_odds テーブル・単勝永続化, PR #56）
- Issue #38（組合せ券種の combination_key 規約・取得）
- 設計書 `docs/specifications/netkeiba-datasource.md` / `predict-session.md` / `backtest.md`

### ADR 0048: ライブオッズ取得を JRA から netkeiba へ統一し odds-scraper を撤去 (Issue #287) (2026-06-28) — 承認済み

#### ステータス

承認済み（ADR 0001 の「ライブ遷移層 `UreqOddsScraper`」を supersede。ADR 0005 / 0010 / 0019 が
前提とする live odds 取得経路を netkeiba へ置換）

#### コンテキスト

`paddock-predict-watch`（発走直前の EV/ROI 監視, #257）のライブオッズ再取得が**全レースで失敗**し、
`response was not valid EUC-JP` 警告とともに `オッズ未取得（未公開/失敗）` でスキップされ、ROI 判定が
一切できない不具合が報告された（#287、2026-06-28 函館・小倉・福島の対象 7 レース全滅）。

切り分けの結果、根本原因は **live odds 取得に 2 系統が併存し、predict-watch 経路だけが
実質機能していない JRA 経路に配線されていた**ことだった。

- JRA 経路（ADR 0001 の `odds-scraper` crate / `UreqOddsScraper`）: `accessO.html` に **`cname`
  セッショントークンを POST** して辿る best-effort 実装で、ADR 0001 時点から**ライブ実地未検証**。
  実際には内部 `RaceId` 文字列を cname として POST するためレスポンスがナンセンスになり、
  それを **EUC-JP 固定デコード**（`scraper_util::decode_euc_jp`）して `response was not valid
  EUC-JP` 警告 → 空オッズに畳まれていた。開催日以外はページ自体が存在しない制約もある。
- netkeiba 経路（`UreqNetkeibaScraper` / `NetkeibaScraper`）: オッズ API
  （`api_get_jra_odds.html?type=1/4/5/6/7/8`）を **UTF-8 JSON** で取得する。#102 / #187 で単複に加え
  組合せ券種（馬連・ワイド・馬単・三連複・三連単）も netkeiba から取得するようになり、fetch-card の
  オッズ保存はこの経路で正常動作している（#287 でも fetch-card は函館5R を 212 件正常保存）。

`OddsInteractor<O: OddsScraper, R>` はジェネリックで、predict / predict-watch / api-server の 3 アプリが
いずれも `O = UreqOddsScraper`（JRA）を注入していた。predict / api-server は read-through キャッシュ
優先（保存済みがあれば再スクレイプしない, ADR 0010）のため fetch-card 保存分でマスクされ顕在化して
いなかったが、キャッシュ無時に live scrape へ落ちると同じ EUC-JP 全滅を起こす**潜在バグ**だった。

#### 決定

1. **`UreqNetkeibaScraper` に `OddsScraper` を実装する。** 内部 `RaceId` を
   `netkeiba_race_id_from_paddock` で 12 桁へ変換し、`fetch_win_place_odds` / `fetch_exotic_odds`
   （UTF-8 JSON 経路）を fetch-card 同様のベストエフォートで呼び、純関数 `assemble_netkeiba` で
   `RaceOdds` に組み立てる。fetch-card と live scrape の取得経路が単一の netkeiba 実装に揃う。
2. **live odds 取得を全アプリで netkeiba へ統一する。** predict / predict-watch / api-server の
   `OddsInteractor` の型引数を `UreqOddsScraper` → `UreqNetkeibaScraper` に差し替える。
3. **JRA `odds-scraper` crate を撤去する。** `UreqOddsScraper` / `OddsPages` / `assemble` /
   JRA odds HTML パーサと fixture を削除し、workspace members・`workspace.dependencies`・各アプリの
   依存からも除去する。use-case の port トレイト `OddsScraper` と `OddsInteractor` のジェネリクスは
   据え置く（実装差し替えを許す抽象として引き続き有効）。

#### 理由

- **根本原因を一度で潰す**: predict-watch だけ差し替えても、predict / api-server に同一の壊れた scraper が
  残る。一時的修正を避け、live odds 取得を実証済みの netkeiba 一本に統一する。
- **取得経路の単一化**: fetch-card（保存）と live scrape（監視・read-through）が同じ netkeiba 実装・
  同じ UTF-8 デコード・同じパーサを共有し、エンコーディング不整合が再発しない。
- **開催日制約の解消**: netkeiba 経路は race_id ベースの GET で確定後も最終オッズを返すため、ADR 0001 の
  「開催日以外はページ自体が存在しない」「ライブ実地未検証」という制約が無くなる。
- **dead code を残さない**: ADR 0001 の JRA 経路は実地で機能しないことが #287 で確定したため、crate ごと
  撤去して将来の誤配線を防ぐ。

#### 影響

- predict-watch の発走直前 ROI 監視が機能する（#287 解消）。predict / api-server の live scrape
  フォールバックも netkeiba 経路になり潜在バグが解消する。
- `assemble_netkeiba` は f64 → `OddsValue`/`PlaceOdds`（finite かつ `>= 1.0`）変換に失敗する行を
  その行だけ skip する（取りこぼし耐性）。組合せ券種は DTO 段階でドメイン型キーを持つためキー変換は不要。
- ADR 0001 / 0005 / 0010 / 0019 が文中で参照する `odds-scraper` / `UreqOddsScraper` は本 ADR 以降
  存在しない（live odds 取得は `UreqNetkeibaScraper` の `OddsScraper` 実装が担う）。これらの ADR は
  歴史的記録として原文を残し、本 ADR で supersede する。
- 検証: 純関数 `assemble_netkeiba` の単体テスト（全券種組み立て・不正行 skip・空入力）に加え、
  #287 で全滅していた函館5R（`2026-1-hakodate-6-5R`）を新 `scrape()` 経路で live 取得し、
  EUC-JP 警告なしに全券種取得できることを確認した（win6/place6/quinella15/wide15/exacta30/trio20/
  trifecta120）。

### ADR 0049: netkeiba オッズ取得の transient リトライと degraded 非0 exit (Issue #288) (2026-06-28) — 承認済み

#### ステータス

承認済み（採用）

#### コンテキスト
`paddock-fetch-card` が netkeiba 単複オッズ API（type=1）の **transient な接続リセット
（Connection reset by peer, os error 54）を「未発売」と同一視して握り潰し**、win_odds=0 のまま
card+近走だけ保存して **exit=0（成功扱い）** で終了していた。結果 `race_odds` に exotic（馬連等）は
入るのに win/place が 0 件になり、`paddock-predict` が当該レースのポートフォリオを生成できず EV/ROI
判定から丸ごと脱落した（2026-06-28 福島・小倉が大量に判定不能になった主因）。間欠的で、リトライすると
回復する（try1 win=0 → try2 win=11 を実測）。

根本原因（コード）:
- `ingest`（`src/use-case/src/interactor/card/ingest.rs`）が `fetch_win_place_odds` の **全エラーを
  握り潰し**空オッズに倒していた。
- scraper では transient は `Error::Fetch`、未発売(status≠result/middle = yoso 等)は `Error::Parse`
  と既に別 variant だったが、`From<Error> for use_case::Error` が **両方 `Internal` に潰し**、
  ingest が区別できなかった。
- netkeiba GET にリトライが無く（タイムアウトのみ）、exit code は常に 0 だった。

ADR 0021（PDF 取得のタイムアウト＋リトライ）/ ADR 0029（jra-fetcher 集約）で確立した transient 判定＋
指数バックオフの policy が `src/interface/jra-fetcher/src/lib.rs` にある。ADR 0048 で odds 経路を
netkeiba に統一済み。

#### 決定
1. **netkeiba GET に transient リトライを追加**（ADR 0021 を netkeiba へ展開）。`scraper.rs` の
   共有 GET ヘルパ `call_with_retry` が `.call()` を transient 失敗時に最大 3 回（初回+2 回）
   指数バックオフ（1s/2s）で再試行する。transient は jra-fetcher の `is_transient` 同様
   `Timeout`/`Io`/`ConnectionFailed`/`HostNotFound`/`Protocol`/5xx。netkeiba は未発売を
   200+JSON status で返すため 403/404=absent 概念は無く、4xx は単純に非 transient。リトライは
   I/O 層の性質として odds 専用にせず `fetch_utf8`/`fetch_decoded` 双方に効かせる。
2. **エラー variant を ingest まで保つ**。`From<Error> for use_case::Error` を
   `Fetch→Fetch` / `Parse→Internal` に変更。
3. **ingest で transient と未発売を分岐**。
   - 未発売(`Internal`): 従来どおり best-effort（card+近走を巻き添えにせず継続、exotic は取れれば保存）。
   - transient(`Fetch`/`Timeout`, リトライ後も残る): **degraded**。win 欠落の部分スナップショット
     （exotic だけ）を永続化すると predict が「オッズ有り・win 無し」で誤判定するため、exotic 取得も
     含め **odds 保存をまるごとスキップ**し `win_odds_degraded` を立てる（cf. #287/commit a54e56b）。
4. **degraded を専用 exit code=3 で surface**。`fetch-card` は近走取り込み（主目的）まで終えた後、
   degraded なら exit code 3 を返す。ハード失敗(=1)と「単複だけ未取得・要再取得」を呼び出し側が
   区別でき、win 欠落レースだけ再取得を回せる。`main` は `std::process::exit` ではなく
   `anyhow::Result<ExitCode>` を返し、tokio ランタイム・DB プール等の Drop を走らせてから終了する。
   既存の消費側 `scripts/predict-check/refresh_ev.sh` は `fetch-card` の exit≠0 を FAIL 扱いして
   「古い DB オッズで評価される」警告を出すため、exit=3 は変更なしで正しく統合される（従来は degraded
   でも exit 0 で "ok" 扱い → 無言で stale オッズ評価していたのが本バグの一面）。

#### 理由
- 「try1 失敗 / try2 成功」の実測どおり、接続リセットの大半はリトライで透過的に回復する。リトライを
  I/O 層に置けば odds 以外（shutuba/近走/payouts）の resilience も同時に上がる（部分対処を避ける）。
- transient と未発売は性質が異なる。未発売は正規の「まだ無い」で握り潰しが正しいが、transient は
  「本来取れるはずが取れていない」ので surface すべき。variant で機械的に分けることで誤判定源を断つ。
- win 欠落の部分永続化は predict のサイレント脱落を生む。保存しない方が「オッズ未取得」として扱われ、
  再取得・次スイープで正される。
- exit=0 のサイレント劣化が運用の誤認を生んでいた。専用コードでハード失敗と区別すれば、呼び出し側は
  win 欠落レースだけを安全に再取得できる。

#### 影響・トレードオフ
- transient 障害時の最悪所要が backoff（1s+2s）分だけ上振れするが、ハングは既存タイムアウトで防止済み。
- degraded 時に exit=3 を返すため、終了コードを見るバルクスクリプト/predict-watch は ≠0 を検知できる
  （意図どおり）。card+近走は保存済みなので主目的の成果は失われない。
- 未発売(yoso)時の挙動は不変（既存テスト 3 本を維持）。

#### スコープ外
- バルク fetch のレート制御（`-j 1 --interval 3 --max-rps 0.3`）は別系統で本 ADR では触らない。
  per-request `delay` + リトライ backoff で本件には十分。
- 取得後の DB count による運用ガードは `win_odds_degraded` フラグ＋非0 exit で実質カバーするため入れない。
- #289（results.trainer の slow query）は別 PR（#296、マージ済み）。

### ADR 0075: 取り込み対象外のレース（障害）は exit 0 + stdout 明示でスキップする (2026-08-11) — 承認済み

#### ステータス

承認済み（本 PR で実装）。対象 Issue: [#586](https://github.com/taito-station/paddock/issues/586)。

#### コンテキスト

`paddock-fetch-card` に障害レースの race_id を渡すと、次のように終わる。

```console
$ paddock-fetch-card 202607020609
Error: internal error: netkeiba parse failed: 障害レースは対応外です
$ echo $?
1
```

障害レースを取り込み対象外とする判断自体は正しい。問題は**呼び出し側が「設計どおりのスキップ」と
「netkeiba 側の実障害」を区別できない**ことにある。

2026-08-09 の開催（3 場 36 鞍）を順次 `fetch-card` するループを回したとき、中京9R（障害）でこの
エラーが出た。ループは `Error:` 行をログに残して次へ進んだが、その 1 件が無視してよい対応外なのか
取り込み失敗なのかは文言を人間が読むまで分からない。結果、`/api/races` の件数（35）と netkeiba
一覧の件数（36）の食い違いを説明するために、欠落 1 件を手で diff して特定する羽目になった。
「対応外だから 1 件少ない」と気づけないと、`--overview` や監視の対象数がずれていても正常と誤認する。

原因はエラー分類にある。`parse/card.rs` の障害判定は `Error::Parse` を返し、`error.rs` の
`From<Error> for paddock_use_case::Error` が**全 `Parse` を `Internal` に潰す**ため、ingest 層より
先では「対応外」であることが失われる。実障害（サイト構造変化）と同じ経路・同じ終了コードになる。

なお近走取り込み（`parse/horse_history.rs`）では、障害・地方・海外を Error にせず行スキップしている。
**同じ「障害」が card 経路だけ Error になっている非対称**が本件の実体であり、
ADR 0049 が transient/未発売を variant で
分けたのと同じ構造をもう一段広げる話になる。

#### 決定

1. `netkeiba_scraper::Error::Unsupported(String)` を新設し、`parse_card` の障害判定はこれを返す。
   馬場・距離表記が読めない場合や `RaceData01` 欠落は従来どおり `Parse`（**「対応外」を広げない**）。
   **障害の判定は距離付きの馬場マーカーで行う**。`SURFACE_DISTANCE_RE` を `(障[芝ダ]?|[芝ダ])\s*(\d{3,4})m`
   とし、交替の先頭に `障` を置いて `障芝` / `障ダ` の複合表記まで 1 トークンとして拾う。単文字クラス
   `[芝ダ障]` だと `障芝3000m` で `障` が候補にならず `芝3000m` にマッチし、**障害レースが芝レースとして
   黙って取り込まれる**（実表記 `障3000m (芝)` では拾えるが表記揺れに脆い）。近走側
   `horse_history::parse_surface_distance` は先頭 1 文字判定で同じ入力を正しく除外しており、card 経路
   だけが弱かった。**テキスト全体の `障` の有無では判定しない**——将来 `障` が別文脈で現れたとき平地
   レースが黙ってスキップされ、実障害の見逃しと同じ害になるため。`match` の `other =>` アームは
   到達不能な防御アームになる（`芝` / `ダ` は通常経路）。
2. `paddock_use_case::Error::Unsupported(String)` を新設する。`From` は理由文字列を前置き無しで渡す
   （利用者向け stdout メッセージにそのまま載せるため）。
3. `CardInteractor::ingest` はこれを捕まえず伝播させ、**カード・オッズ・近走のすべてを打ち切る**。
   `IngestCardResponse` にフラグは足さない。
4. `paddock-fetch-card` は `Unsupported` を捕まえて **理由を stdout に明示し `ExitCode::SUCCESS`**
   を返す。**専用 exit code は新設しない。**

#### 理由

**exit 0 を選ぶ理由。** 「開催なし日付は異常ではないため exit code 0 とし、案内メッセージは stdout に
出力する」（[predict-session.md](../specifications/predict-session.md) の終了コード節）が既に確立した
規約であり、対応外レースも同じく異常ではない。

加えて、**障害レースが実際に到達する消費側は「netkeiba の開催一覧からレースを列挙して回すループ」**
であり（`scripts/predict-check/README.md` の手順が使う `list_races.py`。開催日の全レース取得も同型）、
exit code だけを見てレース単位の成否を判断する。専用 exit code を作ると、対応外レースが取り込み失敗
として計上され、**本 issue の目的（実障害だけを FAIL にする）を達成できない**。exit 0 なら
「exit≠0 = 本物の失敗」と単純に扱える。

ただし正直に記しておくと、**リポジトリ内で自動化されている消費側**（`refresh_ev.sh` / `prefetch_odds.sh`）は
いずれも対象を DB（`race_cards`）由来で作るため障害レースに到達しない。`refresh_ev.sh` は exit≠0 を
一律 FAIL 扱いして「古いオッズ警告」を出すが、これは「exit≠0 を FAIL 扱いする消費側が実在する」
一般例であって本決定の証人ではない。本決定が守るのは、netkeiba 一覧を列挙する手動・半自動のループと、
今後書かれる同型のループである。

**打ち切る理由。** カードが取れない以上、後続の処理は無意味であるだけでなく有害である。

- `race_odds` は `race_cards` への FK を持たない。続行すると**カード無しの孤児オッズ行**が残る
- `parse_shutuba` に障害レースのガードが無いため、`run_history` を走らせると障害レースの出走馬の
  近走取得が**成功してしまい**、取り込まないと決めたレースの馬データが `horses` / `horse_past_runs`
  に入る

`ingest` の `fetch_card` 呼び出しより前に DB 書き込みは無い（`fetch_history_contains` は read-only）
ため、伝播させるだけで「一切書かない」が成立する。

#### 却下した代替案

- **専用 exit code（例 4）を新設する**: 上記のとおり `refresh_ev.sh` が FAIL に計上し目的を達成しない。
  終了コードの語彙を増やす割に、消費側で増える分岐が無い。
- **`IngestCardResponse` に `unsupported` フラグを足す（ADR 0049 の degraded と同型）**: degraded は
  「card 保存済みの部分成功ステータス」なので response フィールドが正しい表現だが、対応外は
  「何もしていない」ので早期打ち切りが正しい。フラグ案は ingest 側でゼロ値レスポンスを捏造し、
  bin 側で後続の println を全部ガードすることになり、行数も分岐も増える。
- **`Parse` のまま bin でメッセージ文字列を照合する**: 文言依存は壊れる。variant で機械的に分けるのが
  ADR 0049 で確立した型。
- **障害レースを取り込めるようにする（`Surface::Jump` の追加）**: ドメイン・確率モデル・predict まで
  波及する。対象外とする判断自体は変えない。

#### 影響

- `netkeiba_scraper::Error` / `paddock_use_case::Error` に variant が 1 つずつ増える。
  `paddock_use_case::Error` の網羅マッチは `rest-controller` の `From<UseCaseError>` のみで、
  そこは `BadRequest`（400）に 1 arm 追加する。**厳密には 422 の方が意味は近い**（リクエスト自体は
  well-formed で、サーバがその資源を扱わない）が、REST 経路からは現状到達しない防御 arm であり、
  クライアント側が扱うステータスを増やさないために既存の 400 に寄せる。実際に露出させる段になったら
  422 へ寄せ直すこと。理由文字列はそのままレスポンス body に載るため、`Unsupported` に内部パス・URL を
  入れない（現状は固定文言のみ）。
- `paddock-fetch-card` がアプリとして返す終了コードは **0 / 1 / 3 のまま**（新設なし。ほかに `clap` 由来の
  引数形式不正 = 2 がある）。障害レースが 1 → 0 へ移る。
- 障害レースを渡した実行は DB を一切変更しない（冪等）。**裏返しとして `fetch_history` にも記録が残らない
  ため、開催日ループを再実行するたびに障害レースの出馬表ページを 1 回取りに行く**。1 開催日あたり数レース
  規模なので許容するが、netkeiba へのペーシングを詰める際はここが対象になる。
- **スキップの識別には stdout の読み取りが要る**（exit code だけでは正常取り込みと区別できない）。
  行頭は `スキップ: ` 固定。**stdout を捨てる消費側からは追えない**——これは受容する。stderr に出すと
  `refresh_ev.sh` が「fetch-card stderr あり」を異常として警告するため、正常な結果を警告に化けさせて
  しまう。`tracing` も `Config::init_tracing` が既定 writer（stdout）で初期化するので代替にならない。
  degraded が stderr なのに対し対象外が stdout なのは、前者が「要再取得」の警告、後者が「正常な結果」
  だからで、この非対称は意図的。

#### スコープ外

- **地方競馬（NAR）の race_id**。CLI は `paddock_race_id_from_netkeiba` で先に解決し、JRA 外の場コードは
  `InvalidArgument` で弾かれる——**HTTP を出す前の引数バリデーションとして exit 1** で終わり、scraper に
  到達しない。仕様書が「不正な race_id は exit 1」と明記済みで、本 ADR は挙動を変えない。障害レースは
  「正しい引数で取得した結果、対象外だと分かった」＝実行時の発見であり、地方は「渡してはいけない引数」
  ＝入力エラー。前者だけが exit 0 に値する。
- `parse/horse_history.rs` の行スキップ（既に対応済み・変更なし）。

### ADR 0086: netkeiba の未発売番兵は払戻倍率として採用しない (2026-08-16) — 承認済み

#### ステータス

承認済み（[#621](https://github.com/taito-station/paddock/issues/621)）。

#### コンテキスト

netkeiba は「未発売・該当なしの組み合わせ」に `99999.9` のような固定値を入れる。これが払戻倍率として
そのまま DB に保存され、EV 計算に流入していた。

```
[ながし] 三連複 3-7-15 ¥200 オッズ99999.9 的中0.1% EV=138.44
ポートフォリオ期待回収率 612.6% / 的中率 10.8% / 賭け計 ¥5000
```

EV は `的中確率 × オッズ`（`leg_metrics`）なので、**1 点で EV が 3 桁**になりポートフォリオの参考 ROI が
跳ね上がる。2026-08-15 の 35 レースでは 10 レースに出現し、参考 ROI が 100% を超えた 3 レースは
すべてこれが原因だった。`predict-watch` の 🔶（≥100%）通知も同じ `compose_portfolio` 経由なので誤発火しうる。

##### 実測で分かったこと

| 項目 | 実測 |
|---|---|
| DB に残っていた番兵は **2 種** | `99999.9`（馬連 / 馬単 / 三連複）と `999999.9`（三連単・32,973 行）。issue は前者のみ挙げていた。**番兵そのものは 3 種**で、ワイドの `9999.9` は保存前に落ちていたため DB に無い |
| 買い目に効く汚染 | `race_odds` に trio 1,599 行 / 70 レース、quinella 156 行 / 14 レース |
| ワイドは 0 行 | **偶然守られていた**（下記） |
| 正当な高配当 | 三連単に `111971.9` / `200886.6` が実在 ＝ 安易な上限は大穴を殺す |
| snapshots | 185,794 行が同じ値を保持（再取得不能資産・#232/#492） |

**ワイドが守られていたのはダミー検知ではない。** netkeiba のワイド番兵は `["9999.9","0.0","--"]` で
両端ともパースできるため `parse_wide_odds` は通り、落ちているのは相方の `odds_high=0.0` が
`OddsValue` の下限（`>= 1.0`）に違反するから。`odds_high` を持たないスカラー券種はこの判定に掛からず
素通りしていた——これが issue の言う「band と scalar の非対称」の実体で、**番兵そのものはどの層でも
見ていなかった**。

#### 決定

1. **番兵値を「オッズではない」として拒否する。判定は特定値の除外**（上限方式ではない）。
   `9999.9` / `99999.9` / `999999.9` を epsilon（`1e-6`）比較で弾く。
   上限を採らないのは、三連単に正当な高配当が実在するため（実測の `111971.9` / `200886.6`）。
   番兵は netkeiba が入れる固定値なので、値を名指しする方が誤爆しない。

2. **判定は `OddsValue::try_from` の 1 か所に置く**（`src/domain/src/odds/odds_value.rs`）。
   `save_race_odds::is_invalid_odds` と `find_race_odds::parse_odds_value` は既にここへ委譲しており、
   スクレイプ経路の `assemble_netkeiba` も同じ変換を通る。したがって保存・読み出し・組み立ての
   全経路に一撃で効き、**band / scalar の非対称も同時に解消**する。
   スクレイパ側にも足すと値域判定が 2 か所になる（ADR 0064 の second source）ので入れない。

3. **既に DB にある番兵行は DELETE しない。読み出しで無害化する。**
   `race_odds_snapshots` は 15 分毎の live オッズを積んだ再取得不能資産で、番兵も「その時点で
   未発売だった」という事実の記録。決定 2 により読み出し時に skip されるため、消さなくても
   EV は正しくなる。

4. **EV 側にはフィルタを足さない。** オッズが無い脚は `bet.odds = None` になり、
   `format_portfolio` の「オッズ未取得」アームと `build_portfolio` の priced フィルタで
   **既に扱われている**。弾くだけで正しい挙動になる。

5. **番兵は `Error::UnpricedSentinel` として値域違反と区別し、ログを `debug` に落とす。**
   番兵は異常ではなく「まだ売れていない」という正常な状態で、1 レースに数百件出る
   （実測: 三連複 560 点中 190 点）。これを warn で出すと本来の値域違反（旧ダンプ由来の残骸など）が
   埋もれるうえ、`--overview` の stdout が 2 万行のログで埋まって下流パーサのノイズになる。

6. **Python の分析経路にも同じ除外を入れる**（`scripts/predict-check/odds_guard.py`）。
   `scripts/` は psql / TSV で DB を直読みするため Rust の値オブジェクトを一切通らない。
   オッズを `float()` 化する入口——`live_ev.parse_exotic` / `live_ev.parse_wide` /
   `umaren_backtest.parse_exotic` / `gate_calibration.load_odds` /
   `snapshot_ev_report.group_snapshots` / `fetch_wide.fetch_wide`——で塞ぐ。これで
   下流（`exotic_mispricing` / `kelly_compare` / `alloc_compare`）も自動的に保護される。

   **ワイド経路を落とさないこと**。`fetch_wide` の `hi < lo` チェックが現状の番兵
   `["9999.9","0.0"]` を弾いているが、それは**相方が 0.0 だから**であって番兵を見ているのではない
   ——本 ADR が「ワイドが守られていたのは偶然」と断じたのと同じ構造なので、`[9999.9, 9999.9]` が
   返れば素通りする。band は**中点化の前**に low / high を個別に見る。

   単勝だけは例外で、`snapshot_ev_report` では**番兵のみ**を落とす。win は「出走馬の確定」にも
   使われるため、下限違反まで落とすと出走馬集合が縮んで ROI の分母が変わる。

7. **番兵リストは言語をまたぐ golden で結ぶ**（`src/domain/src/odds/netkeiba_sentinels.txt`）。
   Rust は `include_str!` してテストで const と突き合わせ、Python は同じファイルを読む。
   同じ値を両言語が別々に持つと片方だけ更新して静かにズレる（ADR 0085 の見出し契約と同型）。

#### 理由

- **特定値の除外が唯一の安全な判定**。券種別の上限は根拠のある閾値を置けず、三連単の正当な
  10 万倍超を落とす。番兵は固定値なので名指しできる。
- **`OddsValue` 一点にするのは既存の設計を踏襲**しただけ。`is_invalid_odds` は当初から
  「値域条件を手書きで複製せず `OddsValue` に委譲する」と書かれており、その単一ソース性が
  そのまま効く。
- **既存行を消さない**のは、snapshots が再取得不能で、かつ「未発売だった」という情報自体に
  価値があるため。読み出しで無害化できる以上、破壊的操作を選ぶ理由がない。
- **ログレベルを分ける**のは、警告の意味を保つため。数百件出る正常状態を warn にすると、
  本当に見るべき値域違反が埋もれる。

#### 却下した案

- **券種別の上限を設ける。** 三連単の正当な高配当（`111971.9` / `200886.6`）を殺す。閾値の根拠も無い。
- **スクレイパ側（`parse/odds.rs`）だけで弾く。** DB に入らなくなるが、**既存の 1,599 行 +
  snapshots 185,794 行は読み出しでそのまま使われ続ける**ため、別途掃除が必須になる。
- **DB の CHECK 制約に上限を足す。** 上限方式の問題に加え、既存行があるとマイグレーションが通らない。
- **既存行を DELETE する。** 再取得不能な履歴を消す。読み出しで無害化できるので不要。
- **EV 計算側でフィルタする。** 「オッズ不明」の扱いが既にあるのに、別の除外規則を EV 層に足すと
  責務が二重になる。

#### 影響

- `paddock_domain::Error` に `UnpricedSentinel` バリアントが増える（`OutOfRange` と区別してログを
  出し分けるため）。
- **既に DB にある番兵行は残るが読まれない。** `race_odds` / `race_odds_snapshots` の掃除は不要。
- 分析スクリプトは `odds_guard.py` を import する。`scripts/predict-check/` の共有モジュールは
  `pred_header.py` に続いて 2 本目。
- **実地確認（2026-08-15 の `--overview` 再実行）**: EV 診断に出ていた `(99999.9)` が 4 箇所 → 0 箇所、
  番兵起因の WARN は 0 行。参考 ROI ≥ 100% の 3 レースは残ったが、**これらは番兵由来ではない**
  （新潟1R はワイド 3-8 の実オッズ 178.4 倍による正当な大穴で、買い目に番兵は 1 点も乗っていない）。
  issue が観測した 612.6% のレースは `race_odds` が PK 上書きのため既に実オッズへ置き換わっており、
  **現在の DB では買い目に乗るケースを再現できない**。その経路は統合テスト
  （`sentinel_odds_row_is_skipped_on_read`）で固定した。
- **誤爆は許容する。** `OddsValue` は券種を知らないので、判定は全券種に一律で効く。したがって
  ワイドの番兵 `9999.9` は三連単の正当な `9999.9` としても拒否されるし、`99999.9` も同様。
  券種別スコープにするには `OddsValue` に bet_type を持ち込む API 変更が要るが、その値ちょうどの
  正当オッズが出る確率と、番兵を通す害（EV が 3 桁）を比べて**誤爆を受け入れる**。
  棄却は `debug` ログなので**誤爆は観測しにくい**点も含めて承知の上の判断。
- **read-through の挙動が両方向に変わる。** `RaceOdds::is_complete()` は券種がそろうことを見るので:
  - （好転）**前日 prefetch の番兵行で complete と誤判定し、当日ずっと番兵を返し続ける**経路が消える
  - （負荷）券種がまるごと未発売の時間帯は `trio` 等が空になり complete に届かず、
    read-through が呼ばれるたびに再スクレイプする。netkeiba へのペーシング規律に触れるので、
    前日プリフェッチを多用する運用では取得回数を見ておく
- **`live_ev_snapshots` の派生 ROI は直らない。** `predict-watch` が**計算済みの ROI と買い目伝票**を
  保存するテーブルで、修正前に書かれた行は番兵起因の ROI をそのまま保持する。`race_odds` と違って
  読み出し時のガードが効かないので、board API 経由では従来の値が見え続ける。
  再計算・掃除の要否は #625（測定の取り直し）と併せて扱う。
- **ADR 0076 / 0079 の測定母集団は汚染されていた**。`race_odds_snapshots` の番兵は trio 7,259 行 /
  111 レース（snapshots を持つ全 486 レースの約 23%）。**汚染がどこに効くかは 2 系統で異なる**:

  - **判定 ROI（`gate_calibration` の `judged_roi`）＝ 汚染の実チャネル**。この値は
    `live_ev_snapshots.roi`、すなわち **predict-watch が Rust で計算して保存済みの ROI**
    （`snapshot.rs` の `ev.roi * 100.0`）をそのまま読んだもの。番兵が `EV = 的中確率 × オッズ` を
    3 桁にする経路はここで、**本 PR の修正でも `odds_guard` でも直らない**（保存済みの数値なので）。
    ADR 0076 の見出し数値（判定ROI 平均 23.2% / 最高 76.8%）はこのチャネル由来。
  - **市場整合 ROI（`market_fair_roi`）＝ ほぼ無影響**。式は `q = (1/o)·W/inv_sum` に対し
    `exp += amount·q·o` なので **`o` が約分されて `amount·W/inv_sum` になる**——番兵脚は
    「ゼロ寄与」ではなく他脚と同額寄与する。番兵の影響は `inv_sum` に `1/999999.9 ≈ 1e-6` を
    足すぶんだけで、しかも `q` を全脚で**下げる**（過小評価方向）ので無視できる。

  したがって**再測定は「odds_guard を入れて計算し直す」では済まない**。判定 ROI は番兵除去後の
  predict-watch が新しく記録し直す必要があり、既存の `live_ev_snapshots` 行は使えない。#625 の
  スコープはそれ（測定のやり直し方そのものが変わる）。

  なお ADR 0076 が「ROI ≥ 100% の通過は 0 件」と結論したこと自体は、その期間の**買い目の脚に
  番兵がほぼ乗らなかった**ことを示唆する（買い目は model top5 の組み合わせ＝人気上位で、番兵は
  売れていない人気薄の組み合わせに出るため）。本 PR の実地確認でも買い目に乗った番兵は 0 点だった。
  ただし最大 76.8% が番兵由来でないと断定はできないので、取り直しの価値は残る。

#### 関連

- [#114](https://github.com/taito-station/paddock/issues/114)（値域ガードの導入。下限のみだった）
- ADR 0076 /
  ADR 0079（参考 ROI の読み方）
- ADR 0085（言語をまたぐ契約を golden で結ぶ前例）
- [docs/qa/QA-odds-sentinel-621.md](../qa/QA-odds-sentinel-621.md)

### ADR 0088: netkeiba の未発売番兵は券種別に判定する (2026-08-18) — 承認済み

#### ステータス

承認済み（[#630](https://github.com/taito-station/paddock/issues/630)・
[#634](https://github.com/taito-station/paddock/issues/634)）。

**ADR 0086 の決定 1 を部分 supersede する**:
「特定値の除外」は維持したまま、判定を**全券種一律**から**券種スコープ**に絞る。決定 2（`OddsValue`
一点判定）・3（既存行は DELETE しない）・4（EV 側にフィルタを足さない）・5（`debug` ログ）・
6（Python 経路の同一ガード）・7（言語をまたぐ golden）は**そのまま維持**する。ADR 0086 自体は
書き換えない。

#### コンテキスト

ADR 0086 は「`OddsValue` は券種を知らない」ため判定を全券種一律とし、**誤爆を承知の上で許容**した
（同 ADR「影響」節）。すなわちワイドの番兵 `9999.9` は三連複・三連単の正当な `9999.9` 配当としても
拒否され、馬連等の番兵 `99999.9` は三連単の正当な `99999.9` としても拒否される。棄却ログが `debug`
のため**誤爆しても観測できない**。

実測（共有 DB・2026-08-18）:

| 項目 | 実測 |
|---|---|
| 番兵値ちょうどの行（`race_odds`） | quinella `99999.9`×156 / exacta `99999.9`×859 / trio `99999.9`×1,599 / trifecta `999999.9`×33,176。**`9999.9` ちょうどは 0 行** |
| 誤爆リスク帯（9000〜11000 の正当配当） | trio **6,244 行** / trifecta **56,230 行** / exacta 699 行 / quinella 48 行 |
| win / place の番兵値行 | **0 行**（両テーブルとも。#634 の答え） |

`9999.9` ちょうどの正当配当は現時点の DB に無いが、trio/trifecta はその前後の帯に数千〜数万行が
恒常的に実在する——**出れば黙って消える**構造で、確率が低いことは観測できないことの言い訳にならない。
さらに Python 側 `snapshot_ev_report.py` は既に単勝だけ番兵判定を別扱いしており、
**Rust（フラット）と Python（win だけ実質券種別）で方針が割れていた**。

#### 決定

1. **番兵は（券種, 値）の組で判定する。** netkeiba の番兵は券種ごとの固定値
   （ワイド `9999.9` / 馬連・馬単・三連複 `99999.9` / 三連単 `999999.9`）であり、判定もその
   スコープで行う。**単勝・複勝に番兵は置かない**（#634。実測 0 行。netkeiba の type=1 応答に
   番兵の観測が無い）。

2. **`impl TryFrom<f64> for OddsValue` を削除し、`impl TryFrom<(BetType, f64)>` に置き換える**
   （`src/domain/src/odds/odds_value.rs`）。券種を渡し忘れた新しい呼び出し口が
   **コンパイルエラーになる**ことが要点で、#621 の失敗様式（ガードを通らない経路が静かに増える）に
   対する唯一の構造的な防御。判定一点主義（ADR 0086 決定 2）はそのまま
   ——`save_race_odds::classify_row` / `find_race_odds::parse_odds_value` / `assemble_netkeiba` は
   券種付きで委譲する。

3. **正本ファイル（`netkeiba_sentinels.txt`）を TAB 区切り `券種<TAB>値` の 2 列にする。**
   番兵を持たない券種（win / place）は**行そのものを置かない**。コメント行は導入しない
   （両言語のパーサ規則を「空行スキップ + split」に保つ）。Rust の golden 突合・Python の
   ローダ・テスト期待値を同じ PR で更新する（ADR 0086 決定 7 の維持）。

4. **Python 公開 API は `is_sentinel(bet_type, odds)` / `is_payout_odds(bet_type, odds)` の
   必須第 1 引数にする**（既定値を持たせない）。未知の券種ラベルは `ValueError` で止める。
   既定値があると呼び出し側の更新漏れが静かに通り、#621 と同じ「例外を出さない壊れ方」を
   再生産するため。

#### 理由

- **番兵は「netkeiba がその券種に入れる固定値」であり、値の同一性は偶然**。ワイドの `9999.9` と
  三連複の `9999.9` は別物で、後者は正当な配当として実在し得る（誤爆リスク帯の実測）。
- **`TryFrom<f64>` を残す折衷案は「ガード漏れが観測できない」を新設する**。券種なし版が残ると
  新規コードがそちらを呼んでも型検査は通り、フラット判定（誤爆）が静かに復活する。
- **コンパイルエラー化は #621 の教訓の直接適用**。#621 の本質は「番兵をどの層も見ていなかった」
  ——見落としを実行時ではなく型で塞ぐ。タプル入力の前例は同ファイルの
  `TryFrom<(OddsValue, OddsValue)> for PlaceOdds`。
- **win/place に空タプルでなく「行なし」を選ぶ**のは、「win に番兵は無い」（値の事実）と
  「win という券種は無い」（ラベルの誤り）を区別するため。前者は正常（`False` を返す）、
  後者はバグ（`ValueError`）。

#### 却下した案

- **`TryFrom<f64>` を残して券種付きを追加する。** 上記の通り、更新漏れが観測できない経路を新設する。
- **フラット判定の維持（ADR 0086 の誤爆許容を続ける）。** 誤爆の確率は低いが、`debug` ログのため
  発生しても観測できず、正当な大穴が黙って消える。Python 側と方針が割れたままにもなる。
- **券種別の上限方式。** ADR 0086 と同じ理由で棄却（正当な高配当を殺す・閾値の根拠が無い）。
- **正本ファイルにコメント行や券種セクションを導入する。** パーサ規則が複雑になり、
  両言語で同一に保つコストが上がる。TAB 2 列で十分。

#### 影響

- **既に DB にある番兵行の無害化（ADR 0086 決定 3）も券種別になる**。三連複の `9999.9` は
  読めるようになり（現時点の DB に該当行は無いので挙動差は将来行のみ）、各券種の番兵は
  引き続き読み飛ばされる。
- **`find_race_odds_morning` は挙動不変**。朝時点の候補は `bet_type='trio'` の DISTINCT
  `fetched_at` で、trio の `99999.9` は引き続き弾かれる。回帰テストは #632/#633（未発売の
  観測記録）側で張る。
- **保存側は未知 `bet_type` ラベルの行を warn+skip する**（従来は券種を見ずに分類・保存し得た）。
  行は ingest が `BetType`(Display) から生成するため、未知ラベルは書き手のバグ。読み出し側の
  「未知は読み飛ばす」（#38）と対になる。
- テストヘルパの `OddsValue::try_from(v)` は全箇所 `(BetType, v)` 化。テスト値は番兵と重ならない
  正当オッズなので、汎用ヘルパは `Win` 固定で包む。
- `snapshot_ev_report.py` の win 分岐（番兵だけを見る）はそのまま残る——win に番兵が無くなるので
  「win の行を番兵以外で落とさない」という意図はより強く満たされる。
- 変異検証済み: 券種条件を落とすフラット退行は Rust（domain 2 件 + gateway `classify_row` 1 件）が
  赤くなり、正本ファイルと定数のズレは Rust golden と Python の双方が赤くなる。

#### 関連

- ADR 0086（決定 1 を本 ADR が券種スコープに絞る。他の決定は維持）
- ADR 0064（second source を作らない——判定一点主義の根拠）
- ADR 0085（言語をまたぐ契約を golden で結ぶ前例）
- [#632](https://github.com/taito-station/paddock/issues/632) /
  [#633](https://github.com/taito-station/paddock/issues/633)（未発売の観測記録。本 ADR の券種付き判定 API の上に乗る）
- [docs/qa/QA-odds-sentinel-scope-630-634.md](../qa/QA-odds-sentinel-scope-630-634.md)

### ADR 0089: 未発売と確認できた券種を観測として記録し、read-through の cache-hit に織り込む (2026-08-19) — 承認済み

#### ステータス

承認済み（[#632](https://github.com/taito-station/paddock/issues/632)）。

**ADR 0010 の「後日談（#294）」で定めた cache-hit 規則を
部分 supersede する**: 「不完全なスナップショットは cache-miss として再スクレイプする」という
原則は維持したまま、**「未発売と確認できた券種の欠落」を不完全さから除外する**。
`RaceOdds::is_complete()` の意味（priced な行が全券種そろっているか）は変えない。ADR 0010 自体は
書き換えない。

ADR 0086 の決定 1/3（番兵はオッズではない・
`race_odds` に入れない）と ADR 0088 の券種スコープ
判定はそのまま維持し、本 ADR はその上に乗る。

#### コンテキスト

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

##### 情報は作れるのに捨てられている

`assemble_netkeiba`（`src/interface/netkeiba-scraper/src/scraper.rs`）は券種ごとに独立したループで
`OddsValue::try_from` を通し、**失敗した行を黙って捨てる**。このとき「netkeiba は行を返したが
1 つも priced にならなかった」＝**未発売と確認できた**という事実が作れるのに、戻り値
`RaceOdds` に載せる場所が無いため失われていた。

一方 `fetch_one_exotic` は取得失敗を空 Vec に畳んでいた（券種単位のベストエフォート・#102）ため、
**「失敗して空」と「未発売で空」が区別できなかった**。この区別が本 ADR の要である。

#### 決定

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
   - **単勝の観測は cache-hit 判定で無視する。** 本番の書き込み経路は組合せ 5 券種しか
     記録しないが、DB の CHECK は語彙統一のため 7 値を許している。仮に `win` の観測行が入ると
     単勝の欠落を免除してしまい、`race_odds()` が win 空のスナップショットを cache-hit で返す
     （fetch-card が degraded 分岐で明示的に避けている「オッズ有り・win 無し」の再現）。
     **複勝は除外しない**——`missing_bet_types()` が `Place` を返さない（`is_complete()` と
     同じ券種集合）ため、観測が混ざっても `missing.is_subset(&fresh)` の結果を変えず、
     除外しても実効が無いから。この不変条件は domain 側の単体テストで固定する。
   - **未来時刻の観測は stale 扱いにする。** 時計のズレやダンプ復元で `observed_at` が未来に
     なると、単純な差分比較では無条件に fresh と判定されて再取得が止まる。gateway が
     「壊れた `observed_at` は読み飛ばす」としているのと向きを揃える。

7. **`RaceOdds::is_complete()` の意味は変えない。** 「priced な行が全券種そろっているか」のまま
   据え置き、cache-hit の判断は use-case 層（`OddsInteractor::race_odds`）が持つ。
   `find_race_odds_morning` の「朝時点＝最初にフル盤が成立した snapshot」（ADR 0088 が
   「挙動不変」と明言・`rest-api-read.md`）がこの意味に依存しているため。
   欠落券種の列挙は `RaceOdds::missing_bet_types()` に集約し、`is_complete()` はそれを使って
   実装する（判定基準の second source を作らない・ADR 0064）。

8. **priced が取れた券種のマークは同一トランザクションで削除する**（DELETE → INSERT の順に
   統一してロック取得順を揃える。逆順の 2 トランザクションが交差するとデッドロックしうる）。 発売が始まったのに
   「未発売」の観測が残ると、次にその券種が一過性失敗で欠けたとき誤って cache-hit してしまう。
   **`save_race_odds` に失敗した回は未発売マークを新しく立てない**——古いスナップショットに
   新しいマークが付くと、TTL のあいだ古い値を cache-hit で返し続ける。**一方マークの削除は
   保存の成否によらず行う**: 削除は「次回取り直す」方向にしか働かないので、見送ると発売開始を
   検知できないまま古い判断が TTL 分残る（どちらの分岐も「迷ったら取り直す」に倒す）。

9. **観測は `predict-watch` 経路（`refresh_race_odds`）でも記録する。** 監視側は cache を見ないが、
   発売開始を最初に観測するのは 5 分毎に回る監視であることが多く、そこでマークを消しておくと
   read-through 側も次回すぐ取り直せる。

#### 却下した案

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

#### 影響

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
    記録するかは follow-up で判断する。**どちらも #649（実地計測）の要件に含めてあり、
    実測の結果を見て打ち手を決める**。

- **恒久的に未発売の券種があるレースでは、priced 済みオッズが最大 TTL ぶん古くなりうる。**
  JRA が三連単等を売らない極小頭数レースでは、修正前は毎回再スクレイプしていたため
  read-through が返す単勝オッズは常にライブだった。修正後は最大 15 分前の保存値を返す。
  判断に使う発走直前オッズは `predict-watch`（read-through を通らない・#257）が担保するので
  実害は無いが、**read-through を通る全経路**——`paddock-predict` 本体の表示値と、api-server の
  `POST /api/sessions/{date}/races/{race_id}/odds:refresh`（手動 refresh）——がこのぶん遅れる。

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

#### 関連

- ADR 0010（read-through / #294 の cache-hit 規則。本 ADR が部分 supersede）
- ADR 0086（番兵はオッズではない。本 ADR の起点となる副作用を予告）
- ADR 0088（券種スコープ判定。本 ADR はその API の上に乗る）
- ADR 0049（netkeiba オッズ経路にレート制御が無いことの裏書き）
- ADR 0068（netkeiba への無駄打ちを構造的に止める規律・IP ブロック）
- ADR 0070（migration の明示適用）
- #633（未発売の記録方針の一貫性）

### ADR 0090: 未発売の記録は「過去の番兵行の保持」と「現在状態の観測」の 2 層で一貫させる (2026-08-22) — 承認済み

#### ステータス

承認済み（[#633](https://github.com/taito-station/paddock/issues/633)）。

**ADR 0086 決定 3 と
ADR 0089 への follow-up**（supersede ではない）。
**実装変更は無い**——両 ADR が既に作った状態を「一貫した立場」として言語化し、混在に見える
非対称の説明を確定知層に固定する決定。

#### コンテキスト

PR #626 のセルフレビュー 2 巡目で指摘され、#633 として据え置かれていた非対称:

- ADR 0086 決定 3 は既存の番兵行を **DELETE しない**理由を「番兵も『その時点で未発売だった』
  という事実の記録だから」と説明した。`race_odds_snapshots` の番兵は **185,794 行**
  （ADR 0086 実測時点の値。マージ当日（2026-08-16）の 16:38 JST まで蓄積が続いたため現在は
  僅かに上回る。15 分毎の live オッズを積んだ**再取得不能資産**・#232）。
- しかし同じ決定の保存ガードにより、**今後は snapshots に番兵が積まれない**。
- つまり字面上は「過去は記録に値する・未来は値しない」に見える。#633 はこれを
  A（未発売も記録に値する→今後も積む）か B（値しない→過去分も掃除）に寄せろと要求した。

その後 ADR 0089（#632）が観測表 `race_odds_unpriced_observations` を新設し、「未発売と確認
できた券種」を記録し始めた。ただしこの表は **現在状態のマーカー**（priced が取れた時点で
DELETE・ADR 0089 決定 8）であり、「いつ未発売で、いつ発売されたか」の時系列記録ではない。

#### 決定

1. **立場は A（未発売は記録に値する）で一貫させる。ただし表現は 2 層で持つ。**
   - **過去分 = `race_odds_snapshots` の番兵生値**。「その時点で未発売だった」という歴史的
     事実の記録として**保持する**（DELETE しない・ADR 0086 決定 3 の維持）。読み出しは
     券種スコープの番兵判定（ADR 0088）で無害化済みなので、残しても EV は汚染されない。
   - **今後分 = `race_odds_unpriced_observations`（ADR 0089）**。「いま未発売と確認できて
     いるか」という現在状態の観測として持つ。**網羅的ではない**——載るのはスクレイプ経路で
     未発売と確認できた券種だけで、fetch-card 自身と全券種未公開の時間帯は記録されない
     （ADR 0089「影響」の残存ケース・#649 の実測で打ち手を判断）。
2. **`race_odds_snapshots` へ番兵（や「未発売フラグ」の行）を積み直すことはしない。**
   snapshots はオッズ（EV 用データ）の履歴であり、未発売は運用観測——**データと観測を
   同じ表に混ぜない**。番兵の生値を積む案は読み出し側ガード頼みへの逆戻りで
   ADR 0086 決定 1 と衝突する（#633 本文が予告していた却下理由そのもの）。
3. **「いつ発売されたか」の時系列が要件になったら、観測表の append-only 化を別 ADR で
   判断する。** 現時点でその時系列の消費者はいない（α 再校正 #218 も発売開始時刻を使わない）。
   [#649](https://github.com/taito-station/paddock/issues/649) の実地計測（前日プリフェッチ帯の
   レスポンス形状の実測を含む）がその判断材料になる。

#### 理由

- **「過去は生値・今後は観測表」は立場の混在ではない。** どちらも未発売を**記録する**——
  過去分は「未発売だった」事実の履歴として残り、今後分は「いま未発売か」の現在状態として
  記録される（発売開始で消える。時系列まで残すかは決定 3 の将来判断）。変わったのは
  **表現と時間軸**（オッズ列に番兵を混ぜる履歴 → 専用表で券種単位に持つ現在状態）であって
  **立場**（記録に値する）ではない。0086 決定 3 の理由文はこの読み方で今後も正しい。
- **B（過去分の掃除）を採らない**のは、snapshots が再取得不能資産であることに加え、
  掃除で得るものが無いため（読み出し無害化済み・分析側も `odds_guard` が除外済み）。
- **表現を過去分に遡って揃えない**（番兵行を観測表形式へ変換して移行する等をしない）のは、
  歴史の書き換えに相当し、変換そのものが新しいバグ面を作るため。層の境界は 2 本の ADR で
  機械的に引ける——**ADR 0086（2026-08-16 マージ）以前 = snapshots の生値 /
  ADR 0089（2026-08-19 マージ）以後 = 観測表**。マージ日は近似で、実効はガードを含む
  バイナリの再ビルド（0086）と共有 DB への migration 明示適用（0089・ADR 0070）の時点で
  前後しうる。間はどちらの表現でも記録されない空白帯だが、帯の大半（8/17 月〜8/19 水）は
  JRA 非開催日で、開催と重なるのは 0086 マージ（8/16 日曜 16:42 JST）後の薄暮開催の残り
  数レースだけ——実データの欠落はあってもその夕方分に限られる（当日の snapshots の番兵は
  16:38 JST が最終行）。

#### 却下した案

- **A の素朴な形: snapshots に今後も番兵（または未発売フラグ行）を積む。** どちらの変形も
  データと観測の混在。**番兵生値**の変形は読み出しガード頼みへの逆戻り（ADR 0086 決定 1 と
  衝突。#468 の値域 CHECK は番兵値を通すため、保存ガードを迂回するだけで再発する）。
  **未発売フラグ行**の変形は `odds NOT NULL` 等の制約緩和が要り、#468 の多重防御を後退させる。
- **B: 既存の番兵行を掃除する。** 再取得不能資産の破壊。得るものが無い。
- **観測表を今すぐ append-only 化して時系列を持つ。** 消費者のいない記録は YAGNI。
  retention・purge 設計（ADR 0089 は「要らない」と判断済み）まで巻き添えで再設計になる。

#### 影響

- **コード・スキーマの変更は無い。** 本 ADR は文書の確定のみ
  （`docs/specifications/netkeiba-datasource.md` の番兵の節と QA へ写す）。
- ADR 0086 決定 3・ADR 0089 の各決定はそのまま有効。
- 将来、観測表を時系列化する場合はこの ADR の決定 3 を起点に別 ADR を切る。

#### 関連

- ADR 0086（決定 1/3。本 ADR が立場を言語化）
- ADR 0088（読み出し無害化の券種スコープ）
- ADR 0089（観測表。本 ADR の「今後分」の実体）
- [#649](https://github.com/taito-station/paddock/issues/649)（実地計測。時系列化の要否の判断材料）
- [docs/qa/QA-odds-record-policy-633.md](../qa/QA-odds-record-policy-633.md)
