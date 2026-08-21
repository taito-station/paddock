# 651 prefetch の lock パスの実測

#651（`prefetch_odds.sh` の lock を `/tmp` 直下の固定パスから UID スコープへ移す）の**調査時点の
生の観測**を残す。issue 本文の転記はしない（ADR 0074）。判断とその理由は
[QA-prefetch-lock-651.md](../qa/QA-prefetch-lock-651.md)、監視系の lock 方針の確定知は
`docs/knowledge/monitor-loop-sleep-resilience.md` が正。

`TMPDIR` が launchd と端末でズレる実測は [585-keep-awake-window-gap.md](585-keep-awake-window-gap.md)
の 1 節が既に持っている（本 issue でも同じ結論を使うが、測り直してはいない）。

## 1. prefetch の lock は pid を記録していない（コード所見・2026-08-22 時点）

`scripts/predict-check/prefetch_odds.sh`（修正前）の lock は 2 経路とも**中身を持たない**。

- flock 経路: `exec 9>"$LOCK"` → `flock -n 9`。ファイルは空のまま。
- mkdir 経路: `mkdir "$LOCK_DIR"` の原子性だけが門。`trap 'rmdir ...' EXIT` で即解放。
  異常終了の残骸は **mtime の時効**（`find "$LOCK_DIR" -prune -mmin +30`）でのみ回収する。

つまり keep-awake（#643）が旧パス移行で使った「lock に記録された pid の生存を見て引き継ぐ」は
**そのままは移植できない**。prefetch 側で生存判定に使える材料は mtime しかない。

## 2. 本番ホスト（macOS）に `flock` は無い＝実運用は mkdir 経路（2026-08-22 実測）

```
$ command -v flock
                          ← 空（不在）
```

- 素の macOS に `flock`(1) は同梱されない。**本番で踏むのは mkdir 経路**。
- ubuntu-latest（CI）は util-linux 同梱で `flock` があり、**flock 経路に入る**。
- したがって「CI が緑」は mkdir 経路が検査された意味にならない。テスト側で PATH を絞って
  両方を踏ませる必要がある。

launchd と端末で**経路が割れる**リスクは低い: plist の `PATH` は
`/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin` で、Homebrew の
`util-linux` を link した場合も端末と同じ位置で見つかる（片方だけ flock 経路になる形にはならない）。

## 3. 敵対 lock で沈黙する経路（コード所見）

他ユーザーが `/tmp/paddock-prefetch.lock.d` を先に作った場合の修正前の挙動:

1. 時効の破棄は `rmdir "$LOCK_DIR" 2>/dev/null || true` ——他人のディレクトリは sticky bit で
   削除できず、**黙って失敗する**。
2. `mkdir` も失敗 → `log "別の prefetch 実行中のためスキップ（mkdir ロック）"` → **`exit 0`**。

結果、launchd から見て毎サイクル成功したまま prefetch が永久に動かない。落とすのは再取得不能な
発走直前 snapshot（#493 が「失敗を exit 0 に握り潰すな」と定めた対象そのもの）。

## 4. lock パスを持つ相方は無い（2026-08-22 実測）

```
$ grep -rn '/tmp/paddock-prefetch\.lock' --include=... .
scripts/predict-check/prefetch_odds.sh:70,71
docs/qa/QA-keep-awake-window-643-585.md:129
```

keep-awake は作成側（`keep_awake.sh`）と削除側（`deployments/launchd/uninstall.sh`）の**2 箇所**が
同じ式を持つ（片方だけ変えると caffeinate を止められなくなる）が、**prefetch の lock パスを持つ
コードは 1 本だけ**。`deployments/launchd/uninstall.sh` は prefetch の launchd エージェントを
unload するが lock には触らない。

ただし `com.paddock.prefetch-odds.plist` のコメントが

> WORKDIR を固定し scratch（lock 等）の場所を launchd と対話シェルで揃える

と書いており、**実態と食い違っていた**（lock は修正前から WORKDIR 非依存の固定パス）。

## 5. 回帰テストの変異確認（2026-08-22）

`scripts/test-prefetch-odds.sh`（14 ケース・1.6 秒・netkeiba / DB に触れない）が本当に対象へ
届いているかを、本体を壊して確かめた。

| 変異 | 落ちたケース |
|---|---|
| lock の既定パスを uid 無しへ戻す | 2 件（静的検査の 2 本） |
| 敵対 lock の `exit 1` を `exit 0` に倒す | 2 件（mkdir 経路 / flock 経路の symlink ケース） |
| mkdir 経路の旧パス回収ブロックを削除 | 3 件（時効内 / 時効切れ / 信用できない旧 lock） |

いずれも変異なしでは 14/14 PASS。macOS には `flock` が無いため、flock 経路の 4 ケースは
`fcntl.flock` を呼ぶ検証用の代替コマンドを PATH に置いて踏ませた（この代替はリポジトリに含めない。
CI の ubuntu では本物の `flock` が使われる）。
