# ADR 0049: netkeiba オッズ取得の transient リトライと degraded 非0 exit (Issue #288)

## ステータス
承認済み（採用）

## コンテキスト
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

## 決定
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

## 理由
- 「try1 失敗 / try2 成功」の実測どおり、接続リセットの大半はリトライで透過的に回復する。リトライを
  I/O 層に置けば odds 以外（shutuba/近走/payouts）の resilience も同時に上がる（部分対処を避ける）。
- transient と未発売は性質が異なる。未発売は正規の「まだ無い」で握り潰しが正しいが、transient は
  「本来取れるはずが取れていない」ので surface すべき。variant で機械的に分けることで誤判定源を断つ。
- win 欠落の部分永続化は predict のサイレント脱落を生む。保存しない方が「オッズ未取得」として扱われ、
  再取得・次スイープで正される。
- exit=0 のサイレント劣化が運用の誤認を生んでいた。専用コードでハード失敗と区別すれば、呼び出し側は
  win 欠落レースだけを安全に再取得できる。

## 影響・トレードオフ
- transient 障害時の最悪所要が backoff（1s+2s）分だけ上振れするが、ハングは既存タイムアウトで防止済み。
- degraded 時に exit=3 を返すため、終了コードを見るバルクスクリプト/predict-watch は ≠0 を検知できる
  （意図どおり）。card+近走は保存済みなので主目的の成果は失われない。
- 未発売(yoso)時の挙動は不変（既存テスト 3 本を維持）。

## スコープ外
- バルク fetch のレート制御（`-j 1 --interval 3 --max-rps 0.3`）は別系統で本 ADR では触らない。
  per-request `delay` + リトライ backoff で本件には十分。
- 取得後の DB count による運用ガードは `win_odds_degraded` フラグ＋非0 exit で実質カバーするため入れない。
- #289（results.trainer の slow query）は別 PR（#296、マージ済み）。
