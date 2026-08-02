---
name: keiba-start
description: >
  paddock で競馬予想セッションを始めるときのスキル。おっちゃん人格への切り替え・起動時のビルド
  最新化とスリープ抑止・データ取得（fetch-card）・オッズ時系列コレクタ起動・予想実行（predict）・
  EV/ROI 判定・買い目決定・ライブ監視の手順を 1 か所に集約。「今日の予想始めます」「予想して」「今日のレース見て」等の
  予想セッション開始発言、または /keiba-start の直接呼び出しで起動する。競馬の実レース予想
  セッションに限る。バックテスト・計測・実装作業では起動しない。
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

> **同一セッションで Step 0.1〜0.3 を実施済みなら、この 3 つを丸ごとスキップして Step 1 へ進む。** このスキルは「予想して」等でセッション途中に再発火しうる。毎回 8 分のビルドと api/Vite 再起動を走らせると、ここで守ろうとした監視窓を自分で潰す。

**何よりも先に現在時刻を見て、Step 0.2 の full build（実測 ~8 分）を挟んでも第 1 レースに間に合うかを判断する。**

発走時刻はこの時点でまだ DB に無い（`post_time` は Step 1 の fetch-card で入る）。当日の発走時刻は netkeiba の開催一覧から取る:

```sh
date "+%Y-%m-%d %H:%M %A"
curl -s -A "Mozilla/5.0" "https://race.netkeiba.com/top/race_list_sub.html?kaisai_date=$(date +%Y%m%d)" \
  | grep -oE 'RaceList_Itemtime">[0-9]{1,2}:[0-9]{2}' | grep -oE '[0-9]{1,2}:[0-9]{2}' | sort | head -1
```

間に合わないときは Step 0.2 のビルドを `--bin` で絞る（後述）。

---

### Step 0.2: ビルド最新化と常駐プロセスの世代リセット（必須）

**前提**: primary チェックアウトの `main` で、作業ツリーがクリーンであること。改変ソースからビルドしたバイナリで当日の張り判断をしないため、散文で済ませず確認する（dirty なら中止して解消する）:

```sh
[ -z "$(git status --porcelain)" ] || echo '⚠ 未コミット変更あり。解消してから進む'
git switch main && git pull --ff-only
```

**先に古い常駐プロセスを落とす。** 長期稼働したプロセスは新しい成果物を配信せず、しかも HTTP 200 を返し続けるので外形からは正常に見える（#570）。**判定基準は「今回のビルドより前に起動したもの」**——「前日以前」ではない。同じ日の朝に起動したプロセスも、リビルド後は等しく古い。

```sh
# 世代を見る（ELAPSED が長い＝古い）。pkill でまとめて落とさない
ps -eo pid,etime,command | grep -E 'paddock-(predict-watch|odds-collect|api)|node.*vite' | grep -v grep

# 落とす前に対象を確認してから pid 指定で落とす
ps -p <pid> -o command=
kill <pid>

# Vite は npm ラッパの子が残るので LISTEN しているものをポートで落とす
lsof -ti tcp:5173 -sTCP:LISTEN | while read -r p; do ps -p "$p" -o command=; kill "$p"; done
```

- **当日分の監視（`predict-watch` / `odds-collect`）が動いているなら落とさない。** 落とすと #568 と同じ「監視が止まったのに気づかない」状態を作る。監視だけは古くても走らせ続け、次のセッション開始時に入れ替える
- 5173 は Vite の汎用既定ポート。**kill する前に `ps -p <pid> -o command=` で本 PJ のものか必ず確かめる**（別プロジェクトの dev server を巻き添えにしうる）

**ビルドする。**

```sh
cargo build --release          # 全バイナリ（実測 ~8 分）
(cd web && npm install)        # フロント依存を追随（npm ci は node_modules を毎回全削除するので dev では install）
```

時間が無ければ `--bin` で絞る。**`paddock-analyze`（直後の migrate に使う）と `paddock-api`（この Step で再起動する）は必ず含める**——欠けると旧バイナリで migrate を打つ／旧 api を「最新」として立て直すことになり、#570 を手順として再現する:

```sh
cargo build --release --bin paddock-analyze --bin paddock-api --bin paddock-fetch-card \
  --bin paddock-predict-watch --bin paddock-odds-collect --bin paddock-predict
```

**バイナリは `~/.local/bin` の symlink 経由で裸コマンドとして叩く。** リンクが primary の `target/release` を指していれば `cargo build --release` だけで全部最新化される。**実体コピーや worktree を指すリンクが混ざると「リビルドしたのに古い成果物が動く」**ので、リンク先まで確認する:

```sh
ROOT=$(git rev-parse --show-toplevel)
ls -l ~/.local/bin/paddock-* | grep -v -- "-> $ROOT/target/release/" \
  && echo "⚠ 実体コピー or 別ツリー参照が混ざっている（要リンク化）" \
  || echo "OK: 全て $ROOT/target/release を指す symlink"
```

```sh
# リンク化（primary チェックアウトから実行する）
ROOT=$(git rev-parse --show-toplevel); mkdir -p ~/.local/bin
for b in "$ROOT"/target/release/paddock-*; do
  [ -f "$b" ] && [ -x "$b" ] && ln -sfn "$b" ~/.local/bin/"$(basename "$b")"
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
(cd web && PADDOCK_API_TARGET=http://127.0.0.1:8080 \
  nohup npm run dev -- --port 5173 --strictPort >> ~/Library/Logs/paddock-vite-$(date +%Y%m%d).log 2>&1 &)
```

**立ち上がりを確認する。**「HTTP 200」は疎通確認にしかならない（#570 のとおり**古いプロセスでも 200 を返す**）。世代の確認は起動時刻で行う:

```sh
for i in $(seq 30); do curl -sf -o /dev/null "http://127.0.0.1:5173/api/live/$(date +%F)" && break; sleep 1; done
for port in 8080 5173; do
  pid=$(lsof -ti tcp:$port -sTCP:LISTEN | head -1)
  [ -n "$pid" ] && ps -o pid,lstart,command -p "$pid" || echo "⚠ :$port が LISTEN していない"
done
```

- Vite の bind は `web/vite.config.ts` が既定で IPv4（`127.0.0.1`）に固定済み（#569 修正済み・上書きは env `PADDOCK_DEV_HOST`）。**`PADDOCK_API_TARGET` の IPv4 明示は依然必要**——proxy 先の既定は `http://localhost:8080` で、Node が `::1` を先に引くと proxy 側だけ ECONNREFUSED になる
- `--strictPort` は旧 Vite が生きていると新 Vite が即死する。**必ず先に 5173 を落としてから**起動する（落とさないと旧 Vite が 200 を返して合格に見える）

**理由**: 2026-08-02 に api-server と Vite が **15 日前起動のまま**残存し、リビルド済みの成果物が反映されていなかった（#570）。DB 側の migration が進まない限り stale binary 警告は出ないため、この種の陳腐化は起動時ガードでは検知できない。

> #570 の恒久対策（`/api/health` にビルド情報を載せる等）が入ったら、この節の手動確認は差し替える。

---

### Step 0.3: スリープ抑止の方針決め（実施は Step 1.6）

**監視は macOS のスリープで止まり、復帰後も再開しない（#568）。** ただし**抑止機構はリポジトリに既にある**（`com.paddock.keep-awake`・#264）。まず load 済みかを確認しておく。

```sh
launchctl list | grep -i paddock     # com.paddock.keep-awake が無ければ未 install
```

- **install/実効確認は Step 1.6 で行う**。`keep_awake.sh` は当日の `post_time` が DB に無いと no-op で終了するため、fetch-card（Step 1）より前に流しても必ず不発になる
- 仕様と限界は [`deployments/launchd/README.md`](../../../deployments/launchd/README.md) の「⚠ スリープ取りこぼしと keep-awake の限界（#264）」が単一ソース（クラムシェル・`pmset` スケジュールスリープ・既にスリープ中の Mac は起こせない）。ここでは再掲しない
- 実害の記録: 2026-08-01 は 14:38 のスリープで監視が止まり、14:50〜18:30 発走の約 12 レースが完全に未監視だった（通知ゼロが「妙味なし」と誤読される静かな失敗）

> #568 の恒久対策（復帰後の自動再開・スイープ途切れの警告）が入ったら、Step 1.6 の手動 fallback は削除する。

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

```sh
pgrep -fl 'paddock-odds-collect --date' && echo '⚠ 既に稼働中。起動しない'
```

起動とスリープ抑止の紐づけは**同一コマンド内で完結させる**。シェル変数は Bash 呼び出しをまたいで残らないため、pid を別ブロックで参照すると空になり、`caffeinate -i -w ""` が黙って失敗する（#568 と同型の静かな失敗）。`caffeinate` は launchd の keep-awake と併用しても無害なので、有無にかかわらず張っておく。

```sh
# バックグラウンド起動（既定: 15分毎・終日・全発走で自動終了）。D は対象開催日
D=YYYY-MM-DD
nohup paddock-odds-collect --date "$D" >> ~/Library/Logs/paddock-odds-collect-${D//-/}.log 2>&1 & \
  nohup caffeinate -i -w $! > /dev/null 2>&1 &
```

```sh
# 稼働確認（ログは追記なので「行数が増えたか」で見る。-s だと前回分で即通過する）
D=YYYY-MM-DD; LOG=~/Library/Logs/paddock-odds-collect-${D//-/}.log
before=$(wc -l < "$LOG" 2>/dev/null || echo 0)
for i in $(seq 30); do [ "$(wc -l < "$LOG")" -gt "$before" ] && break; sleep 1; done
tail -n 3 "$LOG"
pmset -g | grep -i 'prevented'    # "sleep prevented by … caffeinate" が出ること（grep 'sleep' だと常にヒットする）
```

**前提・注意**:
- post_time は `race_cards`（Step 1 の fetch-card 由来）に依存。**fetch-card 済みが前提**（post_time 無しレースは Unknown＝skip）。
- モデル非依存の専用バイナリで predict セッション記録には触れない（確率と買い方の分離を構造で体現・ADR 0055/0060）。
- 予想フロー（Step 2 以降）と並行してバックグラウンドで回す。手動 cron/launchd は不要——おっちゃん起動のたびにここで立ち上げる。
- retention は launchd で自動化済み（手動運用不要）。`com.paddock.purge-snapshots` が毎日 04:30 に `scripts/purge-snapshots.sh`（既定 6 ヶ月保持・`PADDOCK_PURGE_MONTHS` で上書き可）を回し、古い snapshot を `paddock-analyze purge-snapshots` で削除する。`deployments/launchd/install.sh` で常駐配置され `uninstall.sh` では外れない（#492）。

---

### Step 1.6: スリープ抑止を実効化（#568）

fetch-card で `post_time` が DB に入った**後**に実施する（Step 0.3 参照。それより前だと `keep_awake.sh` は no-op で終わる）。

```sh
# リポジトリルートから実行する（plist にリポジトリパスを焼き込むため、worktree から流すと消えるパスを参照し続ける）
"$(git rev-parse --show-toplevel)"/deployments/launchd/install.sh
```

- **副作用**: このスクリプトは keep-awake 単体ではなく **7 エージェントを配置・load** する。うち backup-db（23:30 に共有 DB を dump）/ backup-staleness / verify-backup-restore / **purge-snapshots（毎日 04:30 に古い snapshot を削除）** の 4 本は**常駐で `uninstall.sh` では外れない**。初回実行は削除系を含む恒久ジョブを有効化することを理解した上で流す。keep-awake だけ欲しいなら該当 plist を個別に `launchctl load` する

```sh
# 実効確認（load 確認だけでは不十分。caffeinate が実際に起きているかを見る）
pmset -g | grep -i 'prevented'          # "sleep prevented by … caffeinate" が出ること
tail -n 3 /tmp/paddock-keep-awake/logs/keep-awake.log
```

Step 1.5 と Step 5 の起動ブロックに含めた `caffeinate -i -w $!` は、launchd の有無にかかわらず張っておく（冪等・二重でも無害）。コレクタは朝から終日動くので Step 5 まで無防備にしない。

**開催終了後は片付ける**:

```sh
"$(git rev-parse --show-toplevel)"/deployments/launchd/uninstall.sh   # prefetch / keep-awake / snapshot-coverage の 3 本を外す（常駐 4 本は残る）
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
pgrep -fl 'paddock-predict-watch --date' && echo '⚠ 既に稼働中。起動しない'
```

```sh
# 終日監視（常駐）。起動とスリープ抑止の紐づけを同一コマンド内で完結させる
D=YYYY-MM-DD
nohup paddock-predict-watch --date "$D" >> ~/Library/Logs/paddock-predict-watch-${D//-/}.log 2>&1 & \
  nohup caffeinate -i -w $! > /dev/null 2>&1 &
```

```sh
paddock-predict-watch --date YYYY-MM-DD --once   # 1スイープのみ（cron 等）
```

**バックグラウンド起動にすると通知本文が端末に出ない。定期的にログ本文まで読む**——生存確認だけでは 🔶 買い妙味の通知を取りこぼす（「静かな失敗」を潰すつもりで別の取りこぼしを作らない）。

```sh
D=YYYY-MM-DD; LOG=~/Library/Logs/paddock-predict-watch-${D//-/}.log
grep -E '🔶|🔍' "$LOG" | grep -v 'スイープ: 対象' | tail -20   # ゲート通過の本文（凡例行は除外）
grep 'スイープ:' "$LOG" | tail -1                              # 最終スイープ時刻＝生存確認
pmset -g | grep -i 'prevented'                                 # 抑止が効いているか
```

- スイープ見出しの凡例には 🔶 🔍 が含まれるため、**ゲート通過の抽出では見出し行を除外する**（除外しないと毎スイープ誤検知する）
- 出力される時刻は**スイープ開始時刻**。次の開始までは 間隔（既定 5 分）＋ スイープ所要（`--scrape-delay` 既定 3000ms × 対象レース × 券種）かかるため、多頭数の時間帯は 5 分超が常態。**2×間隔（=10 分）以上空いていたら止まっていると判断する**

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
