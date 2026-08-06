---
name: keiba-start
description: >
  paddock で競馬予想セッションを始めるときのスキル。おっちゃん人格への切り替え・起動時の
  ビルド最新化とスリープ抑止・データ取得（fetch-card）・オッズ時系列コレクタ起動・
  予想実行（predict）・EV/ROI 判定・買い目決定・ライブ監視の手順を 1 か所に集約。
  「今日の予想始めます」「予想して」「今日のレース見て」等の予想セッション開始発言、
  または /keiba-start の直接呼び出しで起動する。競馬の実レース予想セッションに限る。
  バックテスト・計測・実装作業では起動しない。
metadata:
  origin: user
---

# keiba-start

paddock で競馬予想セッションを始めるときのスキル。
おっちゃん人格への切り替え・データ取得・予想実行・買い目決定の手順を 1 か所に集約する。

---

## おっちゃん人格

**このスキルが呼ばれた瞬間から「大阪の気のいい予想屋のおっちゃん」として話す。**
予想セッションが終わる（または開発・実装の話になる）まで、この人格を維持する。

人格の定義（キャラクター設定・スコープ・課題発見時の行動原則・口調サンプル）は
**[persona.md](persona.md) を単一ソース**とする。Step 0 でこのファイルを読み込んで切り替える。

---

## 起動トリガー

「今日の予想始めます」「予想して」「今日のレース見て」など予想セッションの開始を示す発言、または `/keiba-start` の直接呼び出しで起動する。

---

## 手順

### Step 0: 人格切り替え

**[persona.md](persona.md) を読み込み、即座におっちゃん口調に切り替える。** 以降、予想に関するすべての発言はこの口調で行う（開発・実装の話になったら通常のテックリード口調へ戻す）。

---

### Step 0.1: 時刻確認（最優先）

> **同一セッションで Step 0.1〜0.3 と Step 1.6 を実施済みなら、それらを丸ごとスキップして先へ進む。** このスキルは「予想して」等でセッション途中に再発火しうる。毎回 8 分のビルド・api/Vite 再起動・launchd の貼り直しを走らせると、ここで守ろうとした監視窓を自分で潰す。

**何よりも先に現在時刻を見て、Step 0.2 の full build（実測 ~8 分）を挟んでも次のレースに間に合うかを判断する。**

以降のコマンドは **primary チェックアウトを `$ROOT` に固定して実行する**（worktree から流しても primary をビルド・参照するため）:

```sh
ROOT=$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")
```

発走時刻はこの時点でまだ DB に無い（`post_time` は Step 1 の fetch-card で入る）。netkeiba の開催一覧から取るが、**既存パーサを使う**——ad-hoc な grep は second source になり、HTML 構造が変わったときに無言で空を返す（`nk.py` は取得 0 件を warn する）:

```sh
date "+%Y-%m-%d %H:%M %A"
python3 "$ROOT"/scripts/predict-check/upcoming_races.py "$(date +%Y%m%d)" --window-min 1440 | awk 'NR<=3'
```

出力は TAB 区切りで `netkeiba race_id / 発走 HH:MM / 内部 race_id`、発走順に並ぶ。**1 行目が次に発走するレース**。`--all` ではなく `--window-min 1440` を使うのは、`--all` だと発走済みも含むため**午後に始めたセッションで「1 行目＝朝の終了済みレース」を見て誤判断する**ため。

間に合わないときは Step 0.2 のビルドを `--bin` で絞る（後述）。ただし共有依存グラフのコンパイルは削れないので短縮幅は限定的。**それでも間に合わないなら Step 0.2 ごと見送り、既存バイナリのまま監視だけ先に立てる**（古い成果物で回るリスクより、監視窓を落とす方が痛い）。

---

### Step 0.2: ビルド最新化と常駐プロセスの世代リセット（原則必須・例外は Step 0.1）

**前提**: primary の `main` が最新で、追跡ファイルに未コミット変更が無いこと。改変ソースからビルドしたバイナリで当日の張り判断をしないため、散文で済ませず確認する。**⚠ が出たらこれ以降のブロックを実行せず、先に解消する**（各ブロックは別プロセスなので自動では止まらない）:

```sh
if ! git -C "$ROOT" diff --quiet || ! git -C "$ROOT" diff --cached --quiet; then
  echo '⚠ 中止: primary に未コミット変更あり。解消してから進む'
elif ! git -C "$ROOT" switch main || ! git -C "$ROOT" pull --ff-only; then
  echo '⚠ 中止: main への切替 or pull に失敗（他 worktree で checkout 済み等）'
else echo 'OK: primary main が最新'; fi
```

**先に古い常駐プロセスを落とす。** 長期稼働したプロセスは新しい成果物を配信せず、しかも HTTP 200 を返し続けるので外形からは正常に見える（#570）。**判定基準は「今回のビルドより前に起動したもの」**——「前日以前」ではない。同じ日の朝に起動したプロセスも、リビルド後は等しく古い。

```sh
# 世代を見る（ELAPSED が長い＝古い）。pkill でまとめて落とさない
ps -eo pid,etime,command | grep -E 'paddock-(predict-watch|odds-collect|api)|node.*vite' | grep -v grep

# Vite は npm ラッパの子が残るので LISTEN しているものを確認する（まだ落とさない）
lsof -ti tcp:5173 -sTCP:LISTEN | xargs -I{} ps -p {} -o pid=,command=
```

```sh
# 上で本 PJ のものだと確認できた pid だけを指定して落とす
kill <pid>
```

- **当日分の監視（`predict-watch` / `odds-collect`）が動いているなら落とさない。** 落とすと #568 と同じ「監視が止まったのに気づかない」状態を作る。監視だけは古くても走らせ続け、次のセッション開始時に入れ替える
- 5173 は Vite の汎用既定ポート。**kill する前に `ps -p <pid> -o command=` で本 PJ のものか必ず確かめる**（別プロジェクトの dev server を巻き添えにしうる）

**ビルド対象も `$ROOT` に固定する。** cwd 相対で叩くと worktree をビルドしてしまい、symlink は primary を指したままなので「リビルドしたのに古いバイナリが動く」＝#570 の再現になる。

```sh
cargo build --release --manifest-path "$ROOT/Cargo.toml"   # 全バイナリ（実測 ~8 分）
(cd "$ROOT/web" && npm install)   # npm ci は node_modules を毎回全削除するので dev では install
```

時間が無ければ `--bin` で絞る。**`paddock-analyze`（直後の migrate に使う）と `paddock-api`（この Step で再起動する）は必ず含める**——欠けると旧バイナリで migrate を打つ／旧 api を「最新」として立て直すことになり、#570 を手順として再現する:

```sh
cargo build --release --manifest-path "$ROOT/Cargo.toml" \
  --bin paddock-analyze --bin paddock-api --bin paddock-fetch-card \
  --bin paddock-predict-watch --bin paddock-odds-collect --bin paddock-predict
```

- `npm install` は `package-lock.json` を書き換えることがある。差分が出たら**当日はコミットせず**、予想終了後に扱う（上のクリーン判定を自分で壊さないため）

**バイナリは `~/.local/bin` の symlink 経由で裸コマンドとして叩く。** リンクが primary の `target/release` を指していれば `cargo build --release` だけで全部最新化される。実体コピーや worktree を指すリンクが混ざると「リビルドしたのに古い成果物が動く」ので、リンク先まで確認する。`--show-toplevel` は worktree では worktree を返すため、必ず上で定義した `$ROOT` を使う（使い捨て worktree を指すリンクは、消えた瞬間に裸コマンドを全滅させる）。

検証も作成も**対象集合は `cargo metadata` の bin 宣言を単一ソースにする**（削除済みターゲットの残骸を拾わない／拾い損ねない）。**PATH 解決まで確認する**——リンクが正しくても PATH に `~/.local/bin` が無ければ別のバイナリが実行される:

```sh
cargo metadata --no-deps --format-version 1 --manifest-path "$ROOT/Cargo.toml" \
  | python3 -c "import json,sys;[print(t['name']) for p in json.load(sys.stdin)['packages'] for t in p['targets'] if 'bin' in t['kind']]" \
  | while read -r name; do
      link=$(readlink ~/.local/bin/"$name" 2>/dev/null)
      if [ "$link" = "$ROOT/target/release/$name" ] && [ "$(command -v "$name")" = "$HOME/.local/bin/$name" ]; then
        : # OK
      else echo "⚠ $name: link='$link' resolved='$(command -v "$name")'"; fi
    done
echo '（⚠ が 1 行も出なければ全 bin が primary の release を指し PATH でも解決している）'
```

```sh
# リンク化（⚠ が出た場合）
mkdir -p ~/.local/bin
cargo metadata --no-deps --format-version 1 --manifest-path "$ROOT/Cargo.toml" \
  | python3 -c "import json,sys;[print(t['name']) for p in json.load(sys.stdin)['packages'] for t in p['targets'] if 'bin' in t['kind']]" \
  | while read -r name; do
      [ -x "$ROOT/target/release/$name" ] && ln -sfn "$ROOT/target/release/$name" ~/.local/bin/"$name" \
        || echo "⚠ 未ビルド: $name"
    done
```

**migration の確認と適用は分けて実行する**（未適用が無いのに `migrate` を流さない。共有 golden DB への無条件 DDL を避ける）:

```sh
paddock-analyze migrate --dry-run   # まず未適用の有無だけ見る
```

```sh
paddock-analyze migrate             # 上で未適用が出たときだけ流す（CLAUDE.md「DB 運用」）
```

未適用のままだとアプリ側が起動拒否になり、以降の Step が全部止まる。放置しない。

**api-server / Vite を最新で立て直す。**

```sh
nohup paddock-api >> ~/Library/Logs/paddock-api-$(date +%Y%m%d).log 2>&1 &   # 既定 :8080（PADDOCK_SERVER_ADDR で変更可）
(cd "$ROOT/web" && PADDOCK_API_TARGET=http://127.0.0.1:8080 \
  nohup npm run dev -- --port 5173 --strictPort >> ~/Library/Logs/paddock-vite-$(date +%Y%m%d).log 2>&1 &)
```

**立ち上がりを確認する。**「HTTP 200」は疎通確認にしかならない（#570 のとおり**古いプロセスでも 200 を返す**）。世代の確認は起動時刻で行う:

```sh
ok=0
for i in $(seq 30); do curl -sf -o /dev/null "http://127.0.0.1:5173/api/live/$(date +%F)" && { ok=1; break; }; sleep 1; done
[ "$ok" = 1 ] || echo '⚠ Vite→api の proxy が応答しない（PADDOCK_API_TARGET / 旧 Vite 残存を疑う）'
for port in 8080 5173; do
  pid=$(lsof -ti tcp:$port -sTCP:LISTEN | head -1)
  if [ -n "$pid" ]; then ps -o pid,lstart,command -p "$pid"; else echo "⚠ :$port が LISTEN していない"; fi
done
```

- Vite の bind は `web/vite.config.ts` が既定で IPv4（`127.0.0.1`）に固定済み（#569 修正済み・上書きは env `PADDOCK_DEV_HOST`）。**`PADDOCK_API_TARGET` の IPv4 明示は依然必要**——proxy 先の既定は `http://localhost:8080` で、Node が `::1` を先に引くと proxy 側だけ ECONNREFUSED になる
- `--strictPort` は旧 Vite が生きていると新 Vite が即死する。**必ず先に 5173 を落としてから**起動する（落とさないと旧 Vite が 200 を返して合格に見える）
- api/Vite のログは**実行日**で切る（常駐の寿命がセッション単位のため）。監視系（Step 1.5 / Step 5）は**対象開催日**で切る（前夜に仕込む場合があるため）。前夜起動時は両者の日付がずれる

**理由**: 2026-08-02 に api-server と Vite が **15 日前起動のまま**残存し、リビルド済みの成果物が反映されていなかった（#570）。DB 側の migration が進まない限り stale binary 警告は出ないため、この種の陳腐化は起動時ガードでは検知できない。

> #570 の恒久対策（`/api/health` にビルド情報を載せる等）が入ったら、この節の手動確認は差し替える。

---

### Step 0.3: スリープ抑止の方針決め（実施は Step 1.6）

**監視バイナリ自身がスリープ耐性を持つ（#568・ADR 0072）。** `predict-watch` / `odds-collect` は
起動時に自分で `caffeinate -i -w <自分の pid>` を確保し、待機は wall-clock 基準なので**スリープから
復帰すると自動で再スイープし、空いた分を警告する**（`⚠ 前回スイープから N 分空きました…`）。
手動で `caffeinate` を被せる必要はない（**Step 1.5 / Step 5 の起動ブロックに手動 caffeinate を足さない**）。

- **前提: 最新ビルドで起動していること**。#568 以前のバイナリは自己抑止も自動再開もしない。
  ビルド最新化（Step 0.2）を飛ばさない
- launchd の `com.paddock.keep-awake`（#264）は**締切前 prefetch 用**として引き続き必要。load 済みか確認しておく:

```sh
launchctl list | grep -i paddock     # com.paddock.keep-awake が無ければ未 install
```

- **install/実効確認は Step 1.6 で行う**。`keep_awake.sh` は当日の `post_time` が DB に無いと no-op で終了する。plist は `StartInterval=300` なので fetch-card 後 5 分以内に自己回復するが、**確実性を優先して fetch-card の後に実施する**（`keep_awake.sh` は lock+PID で caffeinate の多重起動を防ぐので常時 load でも害はない）
- 仕様と限界は [`deployments/launchd/README.md`](../../../deployments/launchd/README.md) の「⚠ スリープ取りこぼしと keep-awake の限界（#264）」が単一ソース（クラムシェル・`pmset` スケジュールスリープ・既にスリープ中の Mac は起こせない）。ここでは再掲しない
- **蓋を閉じたら全部無効**。2026-08-01 の実害（14:38 のスリープで 14:50〜18:30 発走の約 12 レースが完全に未監視）は 13:23 の clamshell sleep が起点で、`caffeinate -i` の守備範囲外だった。#568 の修正で「復帰後は再開する・途切れは warn される」ようになったが、**寝ている間のスイープは取り返せない**。外出中に監視を当てにするなら蓋を閉じない

---

### Step 1: 開催確認とデータ取得

今日の開催場と race_id を確認する。

```
race_id: 12桁（年2+場2+回2+日2+R2）
場コード: 01=札幌 02=函館 03=福島 04=新潟 05=東京 06=中山 07=中京 08=京都 09=阪神 10=小倉
```

```sh
paddock-fetch-card <12桁race_id>
```

**必須ルール**:
- **fetch-card は必須**。parse-entries（出馬表 PDF）だけだと `horse_past_runs` が空になり前走フォーム特徴量が使えない
- **当日朝は再 fetch**。前日取得時はオッズ未発売（「オッズ: 未確定のため保存なし」と出る）。EV 計算は当日朝の再取得後に行う

**オッズ値域**: `odds=0.0` 等の値域違反は #114 で恒久対処済み・通常は手動対応不要（保存側ガードで INSERT を弾き読み側は warn+skip）。仕組みと残骸掃除の手順は CLAUDE.md「予想ワークフロー > 2. 予想実行」が正。

---

### Step 1.5: オッズ時系列コレクタを起動（バックグラウンド）

**このスキルを読んだら（＝おっちゃん起動時）、fetch-card 済みの当日オッズ時系列コレクタをバックグラウンドで起動する。** 終日 15 分毎に全レースの単複オッズを貯め（`race_odds_snapshots` に append）、"ズレ増額" 判断の実データと将来のオッズ変動分析の母数にする。発走済みは順次対象外・全レース発走で自動終了する。

**二重起動しない。** このスキルはセッション途中に再発火しうる。多重起動は netkeiba への二重スクレイプ（ペーシング違反）と snapshot 重複を招く（`odds-collect` 側に多重起動ガードは無い）。

**ガードは対象開催日まで含めて判定する。** 日付を含めないと、前日から残っているプロセス（#568 で実際に起きた）を「稼働中」と誤判定して**当日の収集が一度も立たない**。

```sh
D=$(date +%F)   # 前夜に翌日分を仕込むときだけ手入力する
pgrep -fl "paddock-odds-collect --date $D" && echo '⚠ 既に稼働中。起動しない' || echo 'OK: 未稼働。起動してよい'
```

既に稼働中でスキップする場合、**それが #568 以前のバイナリなら自己抑止も自動再開も無い**。ログ冒頭に
`── アイドルスリープ抑止を確保しました` **も** `⚠ アイドルスリープ抑止を確保できませんでした` **も**
無ければ古いプロセスなので、止めて最新ビルドで起動し直す（⚠ 側が出ているなら最新ビルドだが
抑止に失敗している＝別の対処が要る）。

```sh
# バックグラウンド起動（既定: 15分毎・終日・全発走で自動終了）。D は対象開催日
# スリープ抑止はバイナリが自分で確保する（#568）。手動 caffeinate は不要
D=$(date +%F)
nohup paddock-odds-collect --date "$D" >> ~/Library/Logs/paddock-odds-collect-${D//-/}.log 2>&1 &
```

```sh
# 稼働確認。ログは追記なので -s も行数比較も前回分で誤判定する。プロセス生存＋今日のスイープ行で見る
D=$(date +%F); LOG=~/Library/Logs/paddock-odds-collect-${D//-/}.log
pgrep -f "paddock-odds-collect --date $D" > /dev/null && echo '生存: OK' || echo '⚠ 起動に失敗している'
tail -n 3 "$LOG"                                        # 起動直後に 1 スイープ出る。時刻が今か確認する
pmset -g | grep -i 'prevented by.*caffeinate' || echo '⚠ caffeinate による抑止が効いていない'
```

- 収集は 15 分間隔なので、初回スイープの後は 30 秒待っても新しい行は出ない。**「行が増えたか」ではなく「プロセスが生きているか＋最終行の時刻」で判断する**

**前提・注意**:
- post_time は `race_cards`（Step 1 の fetch-card 由来）に依存。**fetch-card 済みが前提**（post_time 無しレースは Unknown＝skip）。
- モデル非依存の専用バイナリで predict セッション記録には触れない（確率と買い方の分離を構造で体現・ADR 0055/0060）。
- 予想フロー（Step 2 以降）と並行してバックグラウンドで回す。手動 cron/launchd は不要——おっちゃん起動のたびにここで立ち上げる。
- retention は launchd で自動化済み（手動運用不要）。`com.paddock.purge-snapshots` が毎日 04:30 に `scripts/purge-snapshots.sh`（既定 6 ヶ月保持・`PADDOCK_PURGE_MONTHS` で上書き可）を回し、古い snapshot を `paddock-analyze purge-snapshots` で削除する。`deployments/launchd/install.sh` で常駐配置され `uninstall.sh` では外れない（#492）。

---

### Step 1.6: スリープ抑止を実効化（#568）

fetch-card で `post_time` が DB に入った**後**に実施する（Step 0.3 参照。それより前だと `keep_awake.sh` は no-op で終わる）。

**primary チェックアウトの install.sh を実行する。** plist に焼き込まれるリポジトリパスは「どのコピーのスクリプトを実行したか」で決まる（cwd は無関係）。worktree の install.sh を叩くと、その worktree が消えた瞬間に keep-awake が無言で死ぬ。

```sh
ROOT=$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")   # primary を解決
"$ROOT"/deployments/launchd/install.sh
```

- **副作用**: このスクリプトは keep-awake 単体ではなく **7 エージェントを配置・load** する。うち backup-db（23:30 に共有 DB を dump）/ backup-staleness / verify-backup-restore / **purge-snapshots（毎日 04:30 に古い snapshot を削除）** の 4 本は**常駐で `uninstall.sh` では外れない**。さらに **prefetch-odds（5 分毎に netkeiba をスクレイプ）** も load されるため、odds-collect（15 分）・predict-watch（5 分）と合わせてスクレイプ経路が 3 本になる。削除系を含む恒久ジョブが有効化されることを理解した上で流す
- リポジトリ内の plist は `__REPO_ROOT__` を含む**テンプレート**で、install.sh の `sed` 置換を経て初めて有効になる。**生の plist を個別に `launchctl load` してはいけない**——壊れたパスのジョブが登録され、`launchctl list` には出るのに一度も走らない（本 Step が潰そうとしている静かな失敗そのもの）

```sh
# 実効確認（load 確認だけでは不十分。caffeinate が実際に起きているかを見る）
pmset -g | grep -i 'prevented by.*caffeinate' || echo '⚠ caffeinate による抑止が効いていない'
tail -n 3 /tmp/paddock-keep-awake/logs/keep-awake.log   # このパスは launchd 経由（plist が WORKDIR を固定）のときの位置
```

監視バイナリ側の自己抑止（#568）とこの launchd の keep-awake は**別物で、併用してよい**。前者は監視プロセスの生存期間、後者は締切前 prefetch のタイマー用。二重に `caffeinate` が立っても無害。

**開催終了後は片付ける**:

```sh
ROOT=$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")
"$ROOT"/deployments/launchd/uninstall.sh   # prefetch / keep-awake / snapshot-coverage の 3 本を外す（常駐 4 本は残る）
```

---

### Step 2: 予想実行

```sh
# 全レース一括（通常フロー）
paddock-predict --date YYYY-MM-DD --budget 5000

# 全レースをスキップして predict を完走させ EV 一覧を先に把握したい場合（#479: --skip-all で非対話・stdin 不要）
paddock-predict --date YYYY-MM-DD --budget 5000 --skip-all

# 個別レース確認（ライブ EV 更新・オッズ変動追跡）
# ワイドオッズ（type=5）はこのコマンドが自動取得して EV に反映するため手動取得不要
paddock-analyze predict <race_id> --blend-alpha 0.2
```

- **本番モデル**: 市場単勝 α=0.2 ブレンド・m=10 縮約（`RECOMMENDED_MARKET_BLEND_ALPHA`・CLAUDE.md と一致）

---

### Step 3: EV 判定

ROI = Σ_i(賭金_i × 的中確率_i × 払戻倍率_i) / 総賭金 を算出し、**ROI ≥ 100% のレースだけ張る**。判定基準の詳細（−EV は見送り／+EV は増額可／断然人気の −EV／「高的中・低配当」は無価値）は CLAUDE.md「買い方ルール > レース選択基準」が正。

- **朝時点の判定は仮**。朝の +EV は発走直前に剥がれる（2026-06-27 に全候補剥がれの実害）。**最終 go/no-go は発走直前オッズで再判定**する（下記 Step 5）。ここで買い目を確定させない。

---

### Step 4: 買い目決定

買い方の詳細ルール（予算・配分・混戦判定・軸の選び方・表記規約）は CLAUDE.md「買い方ルール」セクションが正。以下は要点のみ。

| 項目 | 既定値 |
|---|---|
| 予算 | ¥5,000/レース（明示指定がなければ変えない） |
| 3 券種 | ワイド¥1,500 / 馬連¥1,500 / 3連複¥2,000 |
| 相手の広さ | 3 券種とも top5（ワイドも top5・ADR 0065）。詳細は CLAUDE.md「買い方ルール」 |
| 混戦条件 | ◎の勝率 ×0.70 以上の馬が ◎含め 4 頭以上 |

- 混戦時（◎勝率×0.70 以上が 4 頭以上）は配分が変わる → CLAUDE.md「混戦判定と配分」参照
- 買い目は **「式別/方式/軸/相手/点数/金額」のそのまま買える形** で出す
- 馬券は 100 円単位。各レース予算ちょうどに収める
- 表記の実例と用語区別（**ながし / ボックス / フォーメーション**）は CLAUDE.md「買い方ルール > 表記規約（最優先）」参照

---

### Step 5: ライブ監視（発走まで）

発走前レースを定期スキャンし、毎回オッズを再取得して ROI を再計算し、ROI≥ゲートを買い目付きで通知する（Step 3 の朝判定をここで再判定する）。

既定は 窓 40 分 / 間隔 5 分 / ROI ゲート 100% / α=本番 0.2。**常駐起動と `--once` はどちらか一方**を使う。Step 1.5 同様、**既に稼働中なら起動しない**:

```sh
D=$(date +%F)   # 前夜に翌日分を仕込むときだけ手入力する
pgrep -fl "paddock-predict-watch --date $D" && echo '⚠ 既に稼働中。起動しない' || echo 'OK: 未稼働。起動してよい'
```

スキップする場合は Step 1.5 同様、**ログ冒頭の抑止行（`──` 確保 / `⚠` 確保失敗のいずれか）で最新ビルドか確認する**。

```sh
# 終日監視（常駐）。スリープ抑止はバイナリが自分で確保する（#568）。手動 caffeinate は不要
D=$(date +%F)
nohup paddock-predict-watch --date "$D" >> ~/Library/Logs/paddock-predict-watch-${D//-/}.log 2>&1 &
```

```sh
paddock-predict-watch --date YYYY-MM-DD --once   # 1スイープのみ（cron 等）
```

**バックグラウンド起動にすると通知本文が端末に出ない。定期的にログ本文まで読む**——生存確認だけでは 🔶 買い妙味の通知を取りこぼす（「静かな失敗」を潰すつもりで別の取りこぼしを作らない）。

```sh
D=$(date +%F); LOG=~/Library/Logs/paddock-predict-watch-${D//-/}.log
pgrep -f "paddock-predict-watch --date $D" > /dev/null && echo '生存: OK' || echo '停止している（下の終了行で理由を確認）'
grep -E '── .*終了:' "$LOG" | tail -1                          # 終了理由（4 種ある。下記参照）
grep -E '🔶|🔍' "$LOG" | grep -v 'スイープ: 対象' | tail -20   # ゲート通過の本文（凡例行は除外）
grep 'スイープ:' "$LOG" | tail -1                              # 最終スイープ時刻
grep '空きました' "$LOG"                                       # スリープ等で監視が飛んだ区間（#568）
pmset -g | grep -i 'prevented by.*caffeinate' || echo '⚠ caffeinate による抑止が効いていない'
```

- **`⚠ 前回スイープから N 分空きました` が出ていたら、その間に発走したレースは評価されていない**（#568）。
  この行がある日の「通知ゼロ」は**妙味なしの根拠にならない**。途切れの原因（蓋閉じ等）を潰す
- **終了理由は 4 種類あり、意味が違う**。生存確認より先にこれを見る:
  - `発走前のレースが残っていません` = 全レース発走済みの**正常完走**
  - `本日（…）は対象開催がありません` = 開催なし（日付指定ミスも疑う）
  - `全レースで発走時刻（post_time）が不明です` = **fetch-card 未実施**。Step 1 に戻る
  - `対象日（…）を過ぎました` = 日付を跨いだ（#568）。前日から回しっぱなしなら正常な後始末
- スイープ見出しの凡例には 🔶 🔍 が含まれるため、**ゲート通過の抽出では見出し行を除外する**（除外しないと毎スイープ誤検知する）
- 終了行が無いのにスイープが止まっているときだけ「落ちた」と判断する。正常終了をスイープ間隔だけで見ると誤警報になる
- 出力される時刻は**スイープ開始時刻**。次の開始までは 間隔（既定 5 分）＋ スイープ所要（`--scrape-delay` 既定 3000ms × 対象レース × 券種）かかるため、多頭数の時間帯は 5 分超が常態。**正常終了行が無いまま 2×間隔（=10 分）以上空いていたら止まっていると判断する**

predict-watch は **decision-support（判断材料）** で自動 go/no-go ではない。張る/見送り/増額は人間が決め、**軸は監視中も動かさない**。監視中のコミュニケーション規律（毎サイクル冒頭 1 行の現況明示・ズレ警告必須・唯一の正＝最新サイクル・◎ の差し替え禁止）は CLAUDE.md「買い方ルール > ライブ監視時のコミュニケーション規律」および「軸ロックとズレ増額」（ADR 0055・0060）が正。

---

### Step 6: 結果確認

```
https://race.netkeiba.com/race/result.html?race_id=<12桁>
```

（エンコーディング UTF-8。fetch-results はページ生成まで数分〜十数分かかるため手動確認が早い）

---

## 環境メモ

- 予想は primary チェックアウト（main ブランチ）で行う
- 予想中に実装が必要になったら Issue 化だけして予想を続け、実装は別 worktree で並走させる
- `race.netkeiba.com` = UTF-8 / `db.netkeiba.com` = EUC-JP

## 精度実績

最新の数値はメモリ `project_predict_check_workflow.md` を参照。メモリ未ロード時のスナップショット（2026-06-13 実測）: 本命単勝 43.5% / 複勝 65.2% / Top5包含 87%。芝中距離が強み、ダートが弱点。
