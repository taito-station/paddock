# QA: keep-awake の抑止窓の追従（#585）と lock パス（#643）

対象: `scripts/predict-check/keep_awake.sh` / `deployments/launchd/uninstall.sh` の設計判断。
実測は [585-keep-awake-window-gap.md](../original-docs/585-keep-awake-window-gap.md)、
運用上の限界の記述は `deployments/launchd/README.md` が正。

---

## Q1. lock は `${TMPDIR}` 配下と UID スコープの固定パスのどちらにするか

#643 が「**実装前にどちらが同じパスに解決されるか実測する**」と要求していた点。

**実測（2026-08-19）**: 端末の `TMPDIR` は `/var/folders/wq/.../T/`、**launchd は `TMPDIR` 未設定**で
`${TMPDIR:-/tmp}` が `/tmp` に落ちる。`id -u` は両経路とも 501。

**回答: UID スコープの固定パス `/tmp/paddock-keep-awake-$(id -u).lock.d`。**

`${TMPDIR}` を使うと launchd と端末が別 lock を見て互いを見失い、**uninstall が caffeinate を
止められない / keep-awake が二重起動する**という、直そうとしている問題より悪い状態になる。
リポジトリの既存判断（`prefetch_odds.sh`「ロックパスは WORKDIR に依存させず固定にする」）とも一致。

uid を挟むことで `/tmp` 直下の予測可能な固定名という #643 の懸念（他ユーザーが先回りして
ディレクトリを作り lock 取得を妨害できる）は解消する。

反映先: `keep_awake.sh` / `uninstall.sh` の `LOCK_DIR`、`deployments/launchd/README.md`。
担保: `test-keep-awake.sh` の「作成側と削除側が同一の uid スコープ lock パス式を持つ」。

---

## Q2. lock パスの二重定義（作成側 / 削除側）をどう扱うか

`LOCK_DIR` は `keep_awake.sh` と `uninstall.sh` の両方が持つ。ADR 0064 が戒める second source の形。

**回答: 二重定義は残し、機械検査で一致を担保する。**

共有 shell ライブラリはこのリポジトリに存在せず、`LOCK_DIR` 1 つのために新設するのは過剰
（`prefetch_odds.sh` も同じ流儀で固定文字列を持っている）。代わりに
**両ファイルが同一の式を持つことをテストで検査**し、片方だけ変えたら落ちるようにした。
両ファイルに「相方も同時に直すこと」のコメントも置く。

反映先: `test-keep-awake.sh` のケース 8。

---

## Q3. 稼働中 caffeinate の残り時間をどう知るか

**回答: lock に終了時刻（絶対エポック秒）を `end` として記録する。**

**理由**: 残り時間を外部から観測する手段がリポジトリに無い（`ps -o etime` の解析も `pmset` からの
残り時間取得も前例ゼロ。そもそも `pmset -g assertions` は残り時間を持たない）。観測できない以上
記録するしかなく、**既存の `pid` ファイル方式と同型**にするのが素直。

`pid` の**後**に `end` を書く。途中で死ぬと「pid はあるが end が無い」中間状態になるが、これは
「end 不明 → 安全側に倒して張り直す」に落ちるので害がない（逆順だと end だけ新しく残って
古い窓を信じてしまう）。

担保: `test-keep-awake.sh`「延長後の lock に新しい pid と end が記録される」「end 未記入の lock は
安全側に倒して張り直す」。

---

## Q4. 張り直しの順序（新を起動してから旧を落とす / 逆）

**回答: 新を起動してから旧を落とす。** issue #585 の要件「張り直し時は抑止の空白を作らない順序」。

逆順（kill → start）にすると kill から起動までの窓で抑止がゼロになり、**いま直そうとしている
「最大 5 分の空白」を小さく再現する**。一瞬 caffeinate が 2 本重なるが、どちらも `-t` で自動終了
するので無害。旧を落とせなかった場合も、二重に抑止が掛かるだけで実害は無い（warn を出す）。

担保: `test-keep-awake.sh`「起動→停止の順序（抑止の空白を作らない）」。
**変異検査で確認済み**——順序を反転させるとこのテストだけが落ちる。

---

## Q5. 判断できないとき（`end` が読めない・壊れている）はどちらに倒すか

**回答: 延長が必要とみなして張り直す。**

抑止を切らさない側＝安全側。旧形式の lock（`pid` のみ）からの移行もこの経路で自然に吸収される。
#632 で採った「迷ったら取り直す」と同じ向き。

---

## Q6. 旧 lock パスの移行をどう扱うか

**回答: 新パスで lock を取る前に、旧 `/tmp/paddock-keep-awake.lock.d` を 1 回だけ見て回収する。**

これが無いと、修正前の launchd が起動した caffeinate が残ったまま新コードへ切り替わったとき、
新コードは旧 lock を見ないので**二重起動**する（片方は誰も止められない）。生きていれば pid を
引き継いで張り直し、死んでいれば旧ディレクトリを掃除する。移行後は旧パスが二度と作られない。

担保: `test-keep-awake.sh` ケース 7（ただし旧パスが実在する環境では**実運用の caffeinate を
横取りしないよう skip する**）。

---

## Q7. ADR を起票するか

**回答: 起票しない。**

**理由**: 「lock の置き場所」「抑止窓の決め方」を決定として固定した ADR は存在しない
（`keep_awake.sh` のハードコードと README の記述だけ）。ADR 0072 決定 4 は
「スリープ抑止は監視バイナリの責務にせず #264 に一本化」という**責務境界**だけを固定しており、
`keep_awake.sh` 内部の改善はそれと整合する（むしろ「抑止は #264 が担う」という前提を強化する）。
supersede も新 ADR も不要。

**ADR が要るのは次のいずれかを採るとき**（今回は採らない）:
- 抑止を監視バイナリ側に持たせる → ADR 0072 決定 4 の supersede
- `pmset` の wake スケジュールを使う → ADR 0072 の却下案の再評価
- **END の定義そのものを変える** → `snapshot_coverage_check.sh`（#493）が同じ END 基準を共有しており、
  発火タイミングに波及する

本 PR は **END の定義を変えず**、計算済みの END に caffeinate を追従させるだけなので波及しない。

---

## Q8. `prefetch_odds.sh` の同型 lock も直すか

`/tmp/paddock-prefetch.lock.d` も同じく `/tmp` 直下の固定パス。

**回答: 別 issue にする。** #643 のスコープは keep-awake の 2 箇所と明記されており、prefetch の lock は
「二重 fetch 防止」という別の役割・別の壊れ方をする（1 PR = 1 トピック）。

---

## Q9. テストで実 `caffeinate` を起動するか

**回答: 起動しない。PATH 差し替えのスタブを使う。**

実 `caffeinate` を起動するとテストが実機のスリープ抑止に触る。スタブは
`psql`（最終 post_time を返す）/ `caffeinate`（引数を記録して `sleep` に化ける）/
`ps`（`-p PID -o comm=` に `caffeinate` を返す）の 3 つ。

`ps` までスタブするのは、スタブ caffeinate が shebang スクリプトのため**実 `ps` では `comm` が
`bash` になり稼働中判定が成立しない**から。`kill -0` / `kill` は実 pid に対する本物を使う。

**実装中に踏んだ罠**: 偽 caffeinate を `sleep 300 &` で起こす関数を `$( )` の中で呼ぶと、背景
プロセスが置換の stdout パイプを握ってテストが 300 秒固まる。`>/dev/null 2>&1` で fd を落とす。
また **起動有無をスタブのログ行数で判定すると競合する**（`nohup` された背景プロセスの書き込みが
親の終了に間に合わない）ので、`keep_awake.sh` 自身が同期出力する起動ログで判定する。
