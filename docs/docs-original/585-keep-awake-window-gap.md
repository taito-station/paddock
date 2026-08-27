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

## 4. 張り直しで抑止が途切れないことの実測（2026-08-20・実 caffeinate）

回帰テストは `caffeinate` をスタブ化しているため、**実プロセスの受け渡し**は検査していない。
そこで psql だけをスタブにし、`caffeinate` / `ps` / `pmset` は本物のまま 1 回実測した。

手順: 窓を「現在 +3 分」で 1 回目 → 「現在 +6 分」で 2 回目（＝延長が必要な状況）。
その間 `pmset -g assertions` を 0.05 秒間隔でサンプリングし、**自分が起動した 2 つの pid のうち
少なくとも一方が抑止アサーションを保持しているか**を毎サンプル判定する（実機には無関係な
caffeinate も居るので、グローバルな抑止状態では測れない）。

```
1 回目: caffeinate -i -t 180s 起動（pid 98063）… 終了 15:52
2 回目: 抑止窓を延長する: 現行 15:52 → 必要 15:55
        caffeinate -i -t 360s 起動（pid 98228）… 終了 15:55
        旧 caffeinate を停止（pid 98063）
```

サンプルの生データ（3 列目の `95048` は無関係な既存プロセス）:

```
1787208553.534  98063,95048,
...
1787208556.514  95048,98228,
```

**サンプル 32 件中、A(98063)/B(98228) いずれも抑止を持たない瞬間は 0 件**。
pid が入れ替わり、`end` が 1787208720 → 1787208900 へ延び、旧は停止され、新は生存していた。

**この実測が言えないこと**: launchd 経由（`AbandonProcessGroup` 下でジョブを跨いだ caffeinate の
存続）は対象外。同一プロセス内の受け渡しを実プロセスで確かめたにとどまる。
——**この残りは 5 節で解消した**。

**この検証スクリプト自身で 2 度事故った**（どちらも本 PR がコード側で潰した罠と同型）:

- `$END_B）` と書いて全角括弧が変数名に取り込まれ `unbound variable`（#636）。リポジトリ外なので
  `check-shell-var-nonascii.sh` の対象外だった。
- 後片付けが `SAMPLER_PID=0` の状態で `kill 0` を実行し、**プロセスグループ全体（＝実行中のシェル
  自身）を落とした**。`kill 0` は「呼び出し元のプロセスグループ全体」が対象——`pid` を正の整数に
  限定する理由の実例。

## 5. 実 launchd での確認（2026-08-20）

4 節は同一プロセス内の受け渡しまでしか見ていない。残っていた launchd 固有の挙動を、
**使い捨ての LaunchAgent**（`com.paddock.keep-awake-selftest`・`StartInterval=60`）で確認した。

当初は「`post_time` が要るので次開催まで確認できない」としていたが、**未検証だったのは DB ではなく
launchd 経由の挙動**なので、`psql` をスタブにすれば共有 DB に触れずに確認できる。lock / WORKDIR /
ログもすべてスクラッチへ逃がし、実運用の `com.paddock.keep-awake` とはラベルから別にした。

`post_time` を返すスタブの出力を tick の合間に書き換えて「`fetch-card` で鞍数が増えた」状況を作る:

```
17:21:19  caffeinate -i -t 300 起動（pid 1078）… 終了 17:26
17:22:19  既に caffeinate 稼働中（pid 1078）。抑止終了 17:26 は必要窓 17:26 を満たす。延長不要・据え置き
          ← ここで post_time を 17:38 へ
17:23:19  抑止窓を延長する: 現行 17:26 → 必要 17:38
17:23:20  caffeinate -i -t 900 起動（pid 4465）… 終了 17:38
17:23:20  旧 caffeinate を停止（pid 1078）
17:24:20  延長不要・据え置き（安定）
```

確認できたこと:

- **`AbandonProcessGroup=true` が効いている**——ジョブ本体は終了済み（`launchctl list` の PID 欄が `-`）
  なのに caffeinate は生存していた。
- **tick をまたいで lock が持続する**。別プロセスとして起動した次の tick が前回の `pid` / `end` を
  正しく読めている。
- **据え置き判定が実 launchd でも安定する**。60 秒間隔の tick は秒針が必ず異なるので、
  **分丸めが無い実装ならここで「現行 17:26 → 必要 17:26」と誤判定して kill→再起動していた**
  （4 節・QA Q10 の欠陥がまさにこの形で出る箇所）。
- **張り直しで抑止が途切れない**。別の延長（17:38 → 17:49）の前後を `pmset` で 0.05 秒間隔サンプリング
  した結果、**633 サンプル中、旧(4465)・新(11065) いずれも抑止を持たない瞬間は 0 件**。

確認後は unload + plist 削除 + テスト caffeinate 停止まで行い、常駐エージェント 4 本は無傷、
実運用 lock（`/tmp/paddock-keep-awake-<uid>.lock.d`）は作られていないことを確認した。

**なお残る未確認**: 実際の開催日に `fetch-card` の進行で `post_time` が増える経路そのもの
（ここでは DB をスタブで置き換えている）。ただし `keep_awake.sh` から見れば「`MAX(post_time)` が
増える」という同じ入力なので、残差は DB クエリ 1 本ぶん。

## 6. lock パスの散在（調査時点）

`LOCK_DIR="/tmp/paddock-keep-awake.lock.d"` が 2 箇所に**同じ文字列で二重定義**されていた。

| ファイル | 役割 |
|---|---|
| `scripts/predict-check/keep_awake.sh` | 作成側（mkdir・pid 記録・stale 回収） |
| `deployments/launchd/uninstall.sh` | 削除側（pid 読み出し・kill・片付け） |

ドキュメント側に lock パスの記載は無かった（`README.md` / keiba-start SKILL とも WORKDIR/ログの
パスにしか触れていない）。`docs/docs-original/636-fullwidth-after-var.md` に観察記録として
1 度だけ現れるが、これは #636 の一次資料（不変）。
