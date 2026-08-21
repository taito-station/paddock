# QA: prefetch の lock を UID スコープへ移す（#651）

対象: `scripts/predict-check/prefetch_odds.sh` の lock 設計。実測は
[651-prefetch-lock-uid-scope.md](../original-docs/651-prefetch-lock-uid-scope.md)、
確定知は `docs/knowledge/monitor-loop-sleep-resilience.md` が正。

`TMPDIR` を却下して UID スコープの固定パスにする判断そのものは #643 で決着済み
（[QA-keep-awake-window-643-585.md](QA-keep-awake-window-643-585.md) の Q1）。本 QA は
**prefetch 固有の差分**だけを扱う。

---

## Q1. 旧 lock `/tmp/paddock-prefetch.lock{,.d}` の回収方式は何にするか

issue 本文は「生きているプロセスが記録されていれば引き継ぎ、死んでいれば掃除」と書いているが、
**prefetch の lock は pid を記録していない**（mkdir + `trap rmdir` の短命ロック。実測 1 節）。
keep-awake の pid 引き継ぎはそのまま移植できない。

**回答: mtime のみで判定する。** 旧ディレクトリが時効（30 分＝既存の stale 判定と同じ閾値）以内に
触られていれば「移行前のインスタンスが実行中」とみなして `exit 0` で譲り、時効を過ぎていれば残骸と
して `rmdir` する。flock 経路は旧ファイルに `flock -n` を試し、取れなければ譲る。

**pid を記録する案を採らない理由**: prefetch の lock は fetch の間しか存在しない短命ロックで、
`trap` により正常終了でもクラッシュでもプロセス消滅とほぼ同時に解放される。pid を持ち込むと
「pid 未記入の窓」「PID 再利用」「comm 照合」という keep-awake が抱えている分岐を、**それらが
必要になる長命ロックではないのに**丸ごと輸入することになる。移行のためだけに恒久的な状態を
増やす取引に見合わない。

**mtime 判定の弱点は受容する**: 30 分を超えて走り続けている実行中インスタンスは残骸と誤判定される。
ただしこれは**修正前から新パス側に存在する挙動**（`-mmin +30` の stale 破棄）であって、本変更が
持ち込む新しいリスクではない。しかも誤判定が起きるのは旧パスを見る移行期の 1 回だけ。

反映先: `prefetch_odds.sh` の `acquire_lock`。
担保: `test-prefetch-odds.sh` の「旧 lock が時効内 / 時効切れ / 信用できない」3 ケース。

---

## Q2. lock パスが信用できない（symlink / 他ユーザー所有）ときどうするか

修正前は `mkdir` 失敗 → 「別の prefetch 実行中のためスキップ」→ **`exit 0`**。他ユーザーが
`/tmp` に先回りすると、時効破棄の `rmdir` も sticky bit で失敗するため、**外形正常のまま
prefetch が永久に沈黙する**（実測 3 節）。

**回答: `keep_awake.sh` と同じ信用検査（`[ ! -L ] && [ -d ] && [ -O ]`）を入れ、駄目なら大きく
警告して非 0 で終わる。**

- 落とすのは再取得不能な発走直前 snapshot。#493 が「失敗を `exit 0` に握り潰さず launchd へ伝える」
  と定めた対象そのもので、`exit 0` のままにする理由が無い。
- `[ -L ]` を**先に**見る（`-d` / `-O` はリンクを辿るので、自分所有の実体を指す symlink を置かれると
  単独では素通りする）。
- **旧パス側は非 0 にしない**。旧 lock は移行のためだけに見ているので、そこが敵対的でも本来の
  仕事（新パスで lock を取って fetch する）は続けられる。警告を出して移行チェックだけ飛ばす。

**UID スコープ化が防ぐのは事故衝突だけ**という #643 の整理はここでも変わらない。`/tmp` は
world-writable で、悪意ある先回りは uid を挟んでも防げない。だから「防ぐ」ではなく「沈黙しない」に倒す。

反映先: `prefetch_odds.sh` の `abort_untrustworthy_lock`。
担保: `test-prefetch-odds.sh` の symlink ケース（mkdir 経路 / flock 経路の 2 本）。

---

## Q3. 回帰テストで本番経路（mkdir）をどう踏ませるか

`prefetch_odds.sh` は `command -v flock` の有無で lock 機構が変わる。**本番 macOS は flock 不在＝
mkdir 経路、CI の ubuntu は flock 経路**（実測 2 節）。素直にテストを書くと、CI が緑でも
**本番経路は 1 度も検査されない**。

**回答: テスト側で PATH を `$STUB` だけに絞り、`flock` を張らないことで mkdir 経路を ubuntu でも
踏ませる。** `flock` が在る環境では flock 経路のケースも追加で回す（macOS では skip 表示）。

必要な外部コマンド（`bash` `env` `date` `tee` `mkdir` `find` `rmdir` `id` `dirname` `sleep`）だけを
`command -v` で解決して symlink する。`python3` はスタブに差し替え、レース選択には固定 race_id を、
`nk_id` 変換（`python3 - <race_id>`）には**失敗**を返す——開発機には
`target/release/paddock-fetch-card` が実在するので、ここで止めないと**本物の netkeiba スクレイプが
走る**（CI は binary 不在で手前の check に落ちるため、この危険は CI では可視化されない）。

反映先: `scripts/test-prefetch-odds.sh`、`.github/workflows/ci.yml` の `shellcheck` ジョブ。
担保: 変異 3 種（uid 無しへ戻す / 非 0 終了を `exit 0` に倒す / 旧パス回収を削る）で
それぞれ 2・2・3 ケースが落ちることを確認済み（実測 5 節）。

---

## Q4. `$HOME` 配下（`~/Library/Caches/...`）へ移す恒久対処は本 PR でやるか

[QA-keep-awake-window-643-585.md](QA-keep-awake-window-643-585.md) の Q1 が「`prefetch_odds.sh` の
lock（#651）と揃えて別途扱う」として保留した論点。

**回答: 本 PR ではやらない。** `/tmp` を離れれば「他ユーザーの先回り」は構造的に消えるので筋は良いが、
**keep-awake と prefetch の両方を同時に動かさないと意味が無い**（片方だけ移すと「lock は `/tmp` の
uid スコープ固定パス」という流儀が割れ、#643 が避けた中途半端さに戻る）。本 issue の射程は
「#643 と同じ形で prefetch を揃える」ことなので、`$HOME` 移行は 2 スクリプトを束ねた別 issue で扱う。

本 PR で入れた信用検査＋非 0 終了は、`/tmp` に残る限りの緩和として機能する（防げないが沈黙しない）。

---

## Q5. ADR を起票するか

**回答: 起票しない。** UID スコープ化・`TMPDIR` 却下は #643 で決着済みの決定の**適用**であり、
その #643 自身も ADR ではなく一次資料＋QA で記録している（PR #650 は ADR を 1 本も足していない）。
prefetch 固有の差分（mtime 判定・非 0 終了・テストの PATH 絞り）は本 QA と一次資料に残し、
確定知は `monitor-loop-sleep-resilience.md` へ写す。
