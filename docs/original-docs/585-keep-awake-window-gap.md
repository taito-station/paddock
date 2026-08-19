# 585 keep-awake の抑止窓が延長されない / lock パスの実測

#585（抑止窓の空白）と #643（lock パス）の**調査時点の生の観測**を残す。issue 本文の転記はしない
（ADR 0074）。判断とその理由は [QA-keep-awake-window-643-585.md](../qa/QA-keep-awake-window-643-585.md)、
運用上の限界の記述は `deployments/launchd/README.md` が正。

## 1. `TMPDIR` は launchd と端末で解決先が違う（2026-08-19 実測）

#643 が「`${TMPDIR}` を使う場合、launchd ジョブと人が叩く `uninstall.sh` で別のパスを見て互いの
lock を見失うおそれがある——**実装前にどちらが同じパスに解決されるか実測する**」としていた点。

```
$ echo "shell TMPDIR      = ${TMPDIR:-<unset>}"
shell TMPDIR      = /var/folders/wq/f37t3bj56bn3w7m76h49xzx40000gn/T/

$ launchctl getenv TMPDIR
                          ← 空（未設定）

$ id -u
501
```

- 端末: `/var/folders/wq/.../T/`（ユーザー専用ディレクトリ）
- **launchd: `TMPDIR` 未設定** → スクリプトの `${TMPDIR:-/tmp}` が `/tmp` に落ちる

つまり `${TMPDIR}` ベースにすると **launchd は `/tmp/...`、端末は `/var/folders/.../T/...`** を見る。
`id -u` は両経路とも 501 なので、**uid を挟んだ固定パスなら両者が同じ場所に解決される**。

同じ結論はリポジトリ内に先例がある（本 issue 以前から）:

- `scripts/predict-check/prefetch_odds.sh`
  > ロックパスは WORKDIR に依存させず固定にする。launchd は WORKDIR=/tmp/paddock-prefetch、
  > 手動実行は $TMPDIR 配下と WORKDIR が異なるため、WORKDIR 配下に置くと両者が別ロックになり
  > 二重 fetch しうる。
- `deployments/launchd/com.paddock.prefetch-odds.plist`
  > WORKDIR を固定し scratch（lock 等）の場所を launchd と対話シェルで揃える
  > （$TMPDIR は launchd と対話シェルでズレうる）

## 2. 抑止窓が固定されたままになる経路（コード所見・2026-08-19 時点）

`scripts/predict-check/keep_awake.sh`（修正前）:

- 毎サイクル DB から `MAX(post_time)` を引いて `END_MIN = LAST_MIN + BUFFER_MIN` を計算している
- しかし lock に稼働中 caffeinate を見つけると **END を比較せずに即 `exit 0`**
  （`既に caffeinate 稼働中（pid ${pid}）。重複起動せず終了`）
- lock に記録されるのは **`pid` だけ**。現行の抑止終了時刻はどこにも残っていない

→ 起動時に確定した窓が `caffeinate -t` に焼き込まれたまま、後から `post_time` が増えても伸びない。

`deployments/launchd/com.paddock.keep-awake.plist` の `StartInterval` は 300 秒なので、
caffeinate が `-t` で自然終了してから次に張り直されるまで**最大 5 分の抑止空白**が空く。

## 3. 稼働中プロセスの残り時間を外から知る手段はリポジトリに無い（調査時点）

- `caffeinate` の使用は `keep_awake.sh` の 1 箇所のみ。`-w` / コマンドラップ形式の使用例はゼロ
- `ps` の使い方は `-o comm=` のプロセス名照合のみ。`ps -o etime` / `lstart` を**スクリプトが解析している
  箇所は無い**（`.claude/skills/keiba-start/SKILL.md` に人が目視する用途で出てくるだけ）
- `pmset -g | grep -i 'prevented by.*caffeinate'` は**抑止が効いているかの真偽判定だけ**で、
  残り時間は出ない（`pmset -g assertions` も残り時間を持たない）

→ 残り時間は「観測する」のではなく「記録する」しかない、という制約がこの時点で確定した。

## 4. lock パスの散在（調査時点）

`LOCK_DIR="/tmp/paddock-keep-awake.lock.d"` が 2 箇所に**同じ文字列で二重定義**されていた。

| ファイル | 役割 |
|---|---|
| `scripts/predict-check/keep_awake.sh` | 作成側（mkdir・pid 記録・stale 回収） |
| `deployments/launchd/uninstall.sh` | 削除側（pid 読み出し・kill・片付け） |

ドキュメント側に lock パスの記載は無かった（`README.md` / keiba-start SKILL とも WORKDIR/ログの
パスにしか触れていない）。`docs/original-docs/636-fullwidth-after-var.md` に観察記録として
1 度だけ現れるが、これは #636 の一次資料（不変）。
