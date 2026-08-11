# 0075. 取り込み対象外のレース（障害）は exit 0 + stdout 明示でスキップする

## ステータス

承認済み（本 PR で実装）。対象 Issue: [#586](https://github.com/taito-station/paddock/issues/586)。

## コンテキスト

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
[ADR 0049](0049-netkeiba-odds-transient-retry-and-degraded-exit.md) が transient/未発売を variant で
分けたのと同じ構造をもう一段広げる話になる。

## 決定

1. `netkeiba_scraper::Error::Unsupported(String)` を新設し、`parse_card` の障害判定はこれを返す。
   馬場・距離表記が読めない場合や `RaceData01` 欠落は従来どおり `Parse`（**「対応外」を広げない**）。
   なお `SURFACE_DISTANCE_RE` のキャプチャ群は `[芝ダ障]` に限定されているため、`match` の
   「未知の馬場記号」アームは**到達不能な防御アーム**である（正規表現を広げたときのための保険）。
2. `paddock_use_case::Error::Unsupported(String)` を新設する。`From` は理由文字列を前置き無しで渡す
   （利用者向け stdout メッセージにそのまま載せるため）。
3. `CardInteractor::ingest` はこれを捕まえず伝播させ、**カード・オッズ・近走のすべてを打ち切る**。
   `IngestCardResponse` にフラグは足さない。
4. `paddock-fetch-card` は `Unsupported` を捕まえて **理由を stdout に明示し `ExitCode::SUCCESS`**
   を返す。**専用 exit code は新設しない。**

## 理由

**exit 0 を選ぶ理由。** 「開催なし日付は異常ではないため exit code 0 とし、案内メッセージは stdout に
出力する」（[predict-session.md](../specifications/predict-session.md) の終了コード節）が既に確立した
規約であり、対応外レースも同じく異常ではない。

加えて、**障害レースが実際に到達する消費側は「netkeiba の開催一覧からレースを列挙して回すループ」**
であり（`scripts/predict-check/README.md` の手順が使う `list_races.py`。開催日の全レース取得も同型）、
exit code だけを見てレース単位の成否を判断する。専用 exit code を作ると、対応外レースが取り込み失敗
として計上され、**本 issue の目的（実障害だけを FAIL にする）を達成できない**。exit 0 なら
「exit≠0 = 本物の失敗」と単純に扱える。

ただし正直に記しておくと、**リポジトリ内で自動化されている消費側**（`refresh_ev.sh` / `prefetch_odds.sh`）は
いずれも対象を DB（`race_cards`）由来で作るため障害レースに到達しない。本決定が守るのは、上記の
netkeiba 一覧を列挙する手動・半自動のループと、今後書かれる同型のループである。

なお `scripts/predict-check/refresh_ev.sh` も exit≠0 を一律 FAIL 扱いして「古いオッズ警告」を出すが、
同スクリプトの対象レースは `SELECT race_id FROM race_cards` で作られるため、**カードが保存されない
障害レースはそもそも到達しない**。本決定の証人ではなく、「exit≠0 を FAIL 扱いする消費側が実在する」
一般的な例として挙げるに留める。

**打ち切る理由。** カードが取れない以上、後続の処理は無意味であるだけでなく有害である。

- `race_odds` は `race_cards` への FK を持たない。続行すると**カード無しの孤児オッズ行**が残る
- `parse_shutuba` に障害レースのガードが無いため、`run_history` を走らせると障害レースの出走馬の
  近走取得が**成功してしまい**、取り込まないと決めたレースの馬データが `horses` / `horse_past_runs`
  に入る

`ingest` の `fetch_card` 呼び出しより前に DB 書き込みは無い（`fetch_history_contains` は read-only）
ため、伝播させるだけで「一切書かない」が成立する。

## 却下した代替案

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

## 影響

- `netkeiba_scraper::Error` / `paddock_use_case::Error` に variant が 1 つずつ増える。
  `paddock_use_case::Error` の網羅マッチは `rest-controller` の `From<UseCaseError>` のみで、
  そこは `BadRequest` に 1 arm 追加する（REST 経路からは現状到達しないが、到達してもサーバ内部の
  異常ではなく「その資源は扱わない」ため 400 が妥当）。
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

## スコープ外

- **地方競馬（NAR）の race_id**。CLI は `paddock_race_id_from_netkeiba` で先に解決し、JRA 外の場コードは
  `InvalidArgument` で弾かれる——**HTTP を出す前の引数バリデーションとして exit 1** で終わり、scraper に
  到達しない。仕様書が「不正な race_id は exit 1」と明記済みで、本 ADR は挙動を変えない。障害レースは
  「正しい引数で取得した結果、対象外だと分かった」＝実行時の発見であり、地方は「渡してはいけない引数」
  ＝入力エラー。前者だけが exit 0 に値する。
- `parse/horse_history.rs` の行スキップ（既に対応済み・変更なし）。
