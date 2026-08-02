---
name: keiba-start
description: >
  paddock で競馬予想セッションを始めるときのスキル。おっちゃん人格への切り替え・データ取得
  （fetch-card）・オッズ時系列コレクタ起動・予想実行（predict）・EV/ROI 判定・買い目決定・
  ライブ監視の手順を 1 か所に集約。「今日の予想始めます」「予想して」「今日のレース見て」等の
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

### Step 0.4: 時刻確認（最優先）

**何よりも先に現在時刻と第 1 レースの発走時刻を確認し、間に合う対象に絞る。** 以降の Step 0.5 は full build で 8 分前後かかるため、確認せず走らせると最初の監視窓を食い潰す。

```sh
date "+%Y-%m-%d %H:%M %A"
```

間に合わないときは Step 0.5 を crate 限定に絞る（後述）。

---

### Step 0.5: ビルド最新化と常駐プロセスの世代リセット（必須）

**予想セッション開始時は、使うバイナリ・フロントを必ず最新へ作り直す。** 「動いているから大丈夫」で古い成果物を使い続けない。

**前提**: primary チェックアウト（`main`・作業ツリーがクリーン）で実行する。worktree 内や未コミット変更があると `git switch` / `--ff-only` が失敗するので、先に解消してから予想を始める。

```sh
git switch main && git pull --ff-only     # 最新 main を取る（失敗したら解消してから進む）
cargo build --release                     # 全バイナリ（実測 ~8 分）。時間が無ければ
                                          #   cargo build --release --bin paddock-fetch-card \
                                          #     --bin paddock-predict-watch --bin paddock-odds-collect
./target/release/paddock-analyze migrate --dry-run   # 未適用 migration の確認
./target/release/paddock-analyze migrate             # 未適用が出たときだけ適用（CLAUDE.md「DB 運用」）
```

**未適用 migration を放置しない。** 未適用のままだとアプリ側が起動拒否になり、以降の Step が全部止まる。

**バイナリは `./target/release/<bin>` で叩く。** PATH 上の裸コマンドを使わない——`~/.local/bin/paddock-odds-collect` は release からの**実体コピー**で `cargo build` では更新されず、リビルドしたつもりが古い成果物を動かすことになる（他の `paddock-*` は PATH に無い）。

**古い常駐プロセスは落としてから立て直す。** 長期稼働したプロセスは新しい成果物を配信せず、しかも HTTP 200 を返し続けるので外形からは正常に見える（#570）。

```sh
# まず一覧で世代を見る（etime が長い＝古い）。pkill でまとめて落とさない
pgrep -fl 'paddock-predict-watch|paddock-odds-collect'
ps -eo pid,etime,command | grep -E 'paddock-api|node.*vite' | grep -v grep

# 前日以前のものだけ pid 指定で落とす
kill <pid>

# api-server / Vite を最新で立て直す
nohup ./target/release/paddock-api > /tmp/paddock-api.log 2>&1 &        # 既定 :8080（PADDOCK_SERVER_ADDR で変更可）
(cd web && nohup npm run dev -- --host 127.0.0.1 --port 5173 --strictPort > /tmp/paddock-vite.log 2>&1 &)
curl -s -o /dev/null -w '%{http_code}\n' "http://127.0.0.1:5173/api/live/$(date +%F)"   # 200 を確認
```

- **当日分の監視が既に動いているなら落とさない。** このスキルは「予想して」等でセッション途中に再発火しうる。稼働中の `predict-watch` / `odds-collect` を黙って落とすと #568 と同じ「監視が止まったのに気づかない」状態を作る
- Vite の `--host 127.0.0.1` は #569（IPv6 のみに bind して 127.0.0.1 から開けない）の回避。#569 が入ったら不要になる

**理由**: 2026-08-02 に api-server と Vite が **15 日前起動のまま**残存し、リビルド済みの成果物が反映されていなかった（#570）。DB 側の migration が進まない限り stale binary 警告は出ないため、この種の陳腐化は起動時ガードでは検知できない。

> #570 の恒久対策（`/api/health` にビルド情報を載せる等）が入ったら、この節の手動確認は差し替える。

---

### Step 0.6: スリープ抑止（launchd の keep-awake を使う）

**監視は macOS のスリープで止まり、復帰後も再開しない（#568）。** ただし**抑止機構はリポジトリに既にある**（`com.paddock.keep-awake`・#264）。新しく caffeinate を張る前に、まずこれが load されているか確認する。

```sh
launchctl list | grep -i paddock     # com.paddock.keep-awake が無ければ未 install
deployments/launchd/install.sh       # 開催日の朝に流す（prefetch / keep-awake / snapshot-coverage 等）
# 開催日の夜に deployments/launchd/uninstall.sh（常駐エージェントは外れない）
```

仕様と限界は [`deployments/launchd/README.md`](../../../deployments/launchd/README.md) の「⚠ スリープ取りこぼしと keep-awake の限界（#264）」が単一ソース（クラムシェル・`pmset` スケジュールスリープ・既にスリープ中の Mac は起こせない）。ここでは再掲しない。

**launchd を使わないセッション（アドホック起動）では fallback として手動で紐づける。** 対象は predict-watch と odds-collect の**両方**——コレクタは朝から終日動くので、Step 5 まで無防備にしない。

```sh
nohup caffeinate -i -w "$WATCH_PID" > /dev/null 2>&1 &   # pid の取り方は Step 1.5 / Step 5 を参照
pmset -g | grep -i 'sleep'                               # "sleep prevented by caffeinate" を確認
```

- `caffeinate -w` は**対象 pid が既に消えていると即 exit して黙って無効化される**。起動直後に上記 `pmset` で効いていることを必ず確かめる
- 実害の記録: 2026-08-01 は 14:38 のスリープで監視が止まり、14:50〜18:30 発走の約 12 レースが完全に未監視だった（通知ゼロが「妙味なし」と誤読される静かな失敗）

> #568 の恒久対策（復帰後の自動再開・スイープ途切れの警告）が入ったら、この節の手動 fallback は削除する。

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

```sh
# バックグラウンド起動（既定: 15分毎・終日・全発走で自動終了）
# PATH の裸コマンドは古い実体コピーなので使わない（Step 0.5 参照）
COLLECT_LOG=/tmp/paddock-odds-collect-$(date +%Y%m%d).log
nohup ./target/release/paddock-odds-collect --date YYYY-MM-DD > "$COLLECT_LOG" 2>&1 &
COLLECT_PID=$!
nohup caffeinate -i -w "$COLLECT_PID" > /dev/null 2>&1 &   # Step 0.6（launchd 未使用時の fallback）
tail -n 3 "$COLLECT_LOG"                                   # 稼働確認
```

**前提・注意**:
- post_time は `race_cards`（Step 1 の fetch-card 由来）に依存。**fetch-card 済みが前提**（post_time 無しレースは Unknown＝skip）。
- モデル非依存の専用バイナリで predict セッション記録には触れない（確率と買い方の分離を構造で体現・ADR 0055/0060）。
- 予想フロー（Step 2 以降）と並行してバックグラウンドで回す。手動 cron/launchd は不要——おっちゃん起動のたびにここで立ち上げる。
- retention は launchd で自動化済み（手動運用不要）。`com.paddock.purge-snapshots` が毎日 04:30 に `scripts/purge-snapshots.sh`（既定 6 ヶ月保持・`PADDOCK_PURGE_MONTHS` で上書き可）を回し、古い snapshot を `paddock-analyze purge-snapshots` で削除する。`deployments/launchd/install.sh` で常駐配置され `uninstall.sh` では外れない（#492）。

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

```sh
# ログ出力先と pid を確保して起動する（pid もログも無いと Step 0.6 の紐づけと生存確認ができない）
WATCH_LOG=/tmp/predict-watch-$(date +%Y%m%d).log
nohup ./target/release/paddock-predict-watch --date YYYY-MM-DD > "$WATCH_LOG" 2>&1 &
WATCH_PID=$!
nohup caffeinate -i -w "$WATCH_PID" > /dev/null 2>&1 &   # Step 0.6（launchd 未使用時の fallback）

./target/release/paddock-predict-watch --date YYYY-MM-DD --once   # 1スイープのみ（cron 等）
```

既定は 窓 40 分 / 間隔 5 分 / ROI ゲート 100% / α=本番 0.2。

**起動したらスリープ抑止を必ず効かせる**（launchd の keep-awake が load 済みならそれで足りる・未 load なら上記 fallback。#568）。**そのうえで定期的に最終スイープ時刻を確認する**——沈黙は「妙味なし」と「死んでいる」の区別がつかない。

```sh
grep 'スイープ:' "$WATCH_LOG" | tail -1   # 最終スイープ時刻。間隔（既定 5 分）以上空いていたら止まっている
pmset -g | grep -i 'sleep'                # 抑止が効いているか
```

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
