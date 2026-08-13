# paddock プロジェクト CLAUDE.md

## ドキュメント/ナレッジ運用

paddock の文書は HVE（dahatake/HypervelocityEngineering, MIT）の蒸留モデルを取り入れている。
規約の全体は [docs/knowledge/README.md](docs/knowledge/README.md)。

- **3 層**: `docs/original-docs/`（RO 一次資料・生素材 ＋ **ADR**）→ `docs/qa/`（質問票+回答）→
  `docs/knowledge/` ＋ `docs/specifications/`（status 付き確定知）。蒸留は Claude が回す。
- **specifications はその場で knowledge**（frontmatter: `status`/`kind`/`sources`/`distilled_from_sha`/`updated`）。
  frontmatter を付けた時点で確定知層として機能するので、移動する実利が無い。新規の横断的蒸留知は
  `docs/knowledge/` へ。
- **ADR は一次資料層（`docs/original-docs/`）の不変の決定記録**（ADR 0073 で旧 `docs/adr/` から統合）。
  決定を伴う変更は ADR を起票する（採番は `scripts/check-adr-numbers.sh next`）。**一度置いた ADR は
  改変しない**——決定を変えるときは新しい ADR で supersede する。**新設した ADR は同じ PR で
  どこかの knowledge / specifications の `sources` に載せる**（載っていない ADR は
  orphan 検査が error にする・ADR 0082）。写す先が無い ADR（規約そのものを定めた ADR /
  supersede されて下流が後継だけを見るようになった ADR）は `doc-classes.md` の
  `adr-orphan-exceptions` 表に理由付きで登録する。
- **読む入口は knowledge**。ADR の決定・理由・却下案・影響は knowledge に**全部写す**。重複を許す
  代わりに、`sources` の更新に蒸留が追従しているかは機械検査で担保する（人手の規律に委ねない）。
  **写しは一巡済み**: 棄却された ADR は
  [docs/knowledge/product-goals.md](docs/knowledge/product-goals.md) が索引し、採用側はいずれかの
  knowledge / specifications が `sources` で参照している。**「全 ADR がどこかの `sources` に居る」は
  機械検査になった**（ADR 0082。例外は `doc-classes.md` の `adr-orphan-exceptions` 表が正）ので、
  本数を手で数えて書かない。**ただし機械が見ているのは `sources` への登録までで、本文へ写したか
  ではない**。**粒度も一様でない**
  ので、決定の細部（却下案・数値の前提）が要るときは **ADR 原本（`docs/original-docs/0NNN-*.md`）も読む**。
  stale の機械検査は **error**（未解消 6 件を #580 で消化して warning から昇格）。`sources` に挙げた
  ファイルを内容ごと変更したら、参照元の **`distilled_from_sha` を同じ PR で追従させる**（`scripts/bump-distilled-sha.py --all-stale` で一括。`updated` は触らないので実質更新の有無は自分で判断する。追従漏れは
  CI が落とす）。`updated` は**下流の本文が実質変わったときだけ**進める（機械検査の対象外）。
- **用語で迷ったら [docs/knowledge/glossary.md](docs/knowledge/glossary.md)（D07）**。`win_prob` の
  スケール・`blended` の α・`軸ロック` / `混戦` / `ながし` などの**定義の正本がどこにあるか**を引ける
  索引で、定義そのものは各仕様書・ADR・本ファイルが持つ。
- **`docs/original-docs/` の命名は 2 系統**: ADR = **0 埋め 4 桁**（`0055-...`）/ issue 由来の一次資料 =
  **issue 番号・0 埋めしない**（`382-...`）。これが ADR 番号重複検出の判定根拠なので破らない。
- **status**: `Confirmed`（運用の前提にしてよい）/ `Tentative`（暫定）/ `Conflict`（矛盾・放置せず解消）。
- **文書クラス**: knowledge/specifications は frontmatter に `doc_class`（+ mdq 用ミラーの `tags`）を持つ。
  定義の正本は [docs/knowledge/doc-classes.md](docs/knowledge/doc-classes.md)（HVE の D01〜D21 ＋ paddock 固有の
  D22 予測モデル / D23 買い方 / D24 実験・棄却証跡）。`scripts/mdq search --tags D23` でクラス絞り込みができる。
  整合は `scripts/check-doc-classes.py` が CI と pre-push で検査する（本文の相対リンクの実在、`doc-classes.md` の割当索引との 1 対 1 突合、**REQ 表の `出典` が `sources` にも載っているか**、**どの `sources` からも参照されない ADR の検出**——いずれも error）。**knowledge / specifications を 1 本足す・消す・`doc_class` を変えるときは、同じ PR で `doc-classes.md` のクラス一覧の「現行」列と割当索引も直す**。
- **探索規律 — 生読み前に mdq 検索**: docs 内の答えを探すときは、まず
  `scripts/mdq search --q "..."`（BM25・ローカル・[.claude/skills/markdown-query/SKILL.md](.claude/skills/markdown-query/SKILL.md)）
  でヒットチャンクだけ取り、必要時のみ生ファイルへ。コード探索は従来通り serena（`mcp__serena__*`）。
  索引 `.mdq/` は gitignore・セッション毎に `scripts/mdq index` で再ビルド（初回は
  `python3 -m venv tools/mdq/.venv && tools/mdq/.venv/bin/pip install -r tools/mdq/requirements.txt`）。
  **ADR 統合（ADR 0073）より前の索引を持つ環境は一度だけ `rm -rf .mdq && scripts/mdq index`**
  で作り直す。増分の prune は roots 配下しか消さないため、旧 `docs/adr/*` のチャンクが居残って
  存在しないパスが検索結果に出続ける。

## DB 運用

- **migration 追加後は共有 DB へ `paddock-analyze migrate` で明示適用する**（起動時は自動適用されない・#470/ADR 0070）。`--dry-run` で未適用一覧のみ確認できる。
- app 起動時は既定で auto-migrate しない（read-only 整合チェックのみ）。未適用/未初期化 DB は起動が停止するので `paddock-analyze migrate` を先に流す。**stale binary の warn**（DB が先行）が出たら最新ブランチで再ビルドする。

## 予想ワークフロー

競馬予想を行うセッションでは以下の手順で進める。毎回この流れを守る。

### 1. データ取得

```sh
# 開催特定: race_id は 12 桁（年2+場2+回2+日2+R2）
# 場コード: 02=函館 05=東京 06=中山 08=京都 09=阪神 10=小倉 etc.
paddock-fetch-card <12桁race_id>
```

- **fetch-card は必須**。parse-entries（出馬表 PDF）だけだと近走が埋まらず前走フォーム特徴量が使えない。
- fetch-card が完走すると card + 単勝オッズ + 近走（horse_past_runs）がすべて DB に入る。
- 前日取得時はオッズ未発売で「オッズ: 未確定のため保存なし」になる。EV 計算は当日朝に再取得してから。
- **当日は fetch-card 後にオッズ時系列コレクタをバックグラウンド起動**し、終日 15 分毎に全レースの単複オッズを `race_odds_snapshots` に貯める（"ズレ増額" 判断の実データ・全レース発走で自動終了。keiba-start スキル非発火のセッションでも忘れず立てる）:

```sh
paddock-odds-collect --date YYYY-MM-DD   # fetch-card 済みが前提（post_time 依存）・15分毎・終日
```

- **起動はスリープ抑止とセットで行う**（macOS がスリープすると監視・収集が止まり復帰後も再開しない・#568）。ログ出力先・`caffeinate` の紐づけ・二重起動ガードを含む具体手順は [.claude/skills/keiba-start/SKILL.md](.claude/skills/keiba-start/SKILL.md) の Step 1.5 / 1.6 / 5 が正。スキル非発火のセッションでもこの手順に従う。
- **セッション開始時はバイナリを最新化する**（`~/.local/bin/paddock-*` は primary の `target/release` への symlink。`cargo build --release` で全部最新になる）。長期稼働した api-server / Vite が古い成果物を配信し続ける問題を含め、手順は同スキル Step 0.1〜0.2 が正（#570）。

### 2. 予想実行

```sh
# 1日全レースを予想（通常の予想フロー）
# 対話起動し、各レースのプロンプトで s+Enter（スキップ）か確認入力を繰り返す
paddock-predict --date YYYY-MM-DD --budget 5000

# 一括スキップ（予想・買い目推奨だけ流して確認）: --skip-all で非対話（stdin を一切読まない・#479）
# 全レース s 相当・馬場はデフォルト採用（プロンプトなしで表示）・買い目は記録しない
# （馬場条件だけは #80 に従い対話時同様に保存されうる。買い目 bet_records のみ非記録）
paddock-predict --date YYYY-MM-DD --budget 5000 --skip-all

# EV 一覧の再表示（#551）: --skip-all の一過性 stdout を後から何度でも見返す
# 完了済みセッションでも当日オッズで EV 一覧（確率・買い目推奨・回収率）を再計算して表示する。
# 予想セッション状態（セッション・買い目・馬場条件）は書き込まない（predict_sessions の手動 DELETE 不要）。
# オッズは skip-all 同様 read-through（不完全キャッシュのレースは再スクレイプし race_odds を更新しうる）。
# 予算上限は各レース --race-budget（残高で絞らない）。--budget は不要。
paddock-predict --date YYYY-MM-DD --overview

# 個別レースのモデル勝率確認（EV 算出・オッズ確認時）
# ワイドオッズ（netkeiba type=5）はこのコマンドが自動取得して EV に反映するため手動取得不要
paddock-analyze predict <race_id> --blend-alpha 0.2
```

- 本番モデル: 市場単勝 α=0.2 ブレンド・m=10 縮約。
- race_odds の `odds=0.0` 等の値域違反は #114 で恒久対処済み（手動 DELETE は不要）。保存側 `save_race_odds` が `OddsValue::try_from`（有限かつ ≥1.0）委譲ガードで無効行の INSERT を弾き、読み側 `find_race_odds` は値域違反のスカラー行を warn+skip して継続する（predict は全停止しない）。band 券種（ワイド等）の構造不正〈`odds_high` NULL / low>high〉のみ意図的に stop する（残骸ではなくバグ検知）。golden DB に残骸は無く、旧 SQLite 由来のダンプを取り込む等で残骸を抱えた DB に限り `DELETE FROM race_odds WHERE odds < 1.0` で掃除する。

### 3. EV 判定 → 買い目決定

> ROI の定義は用語集（[docs/knowledge/glossary.md](docs/knowledge/glossary.md)）が本節から写している。
> 変えたら同ファイルの ROI 行も見直すこと（機械検査は鳴らない・ADR 0077）。

各レースの ROI = Σ_i(賭金_i × 的中確率_i × 払戻倍率_i) / 総賭金 を算出する。**ただしこの参考ROIをレース選別のゲートとして当てにしない**（182R の実測で ROI ≥ 100% の通過 0 件・実現ROIへの選別力なし・ADR 0076）。判定基準は下記「レース選択基準」参照。

**朝の +EV は発走直前に剥がれる**（市場が締まると妙味が消える）。EV/ROI 判定は発走直前のフレッシュなオッズで行う。これを自動化する監視コマンド:

```sh
# 発走前レースを定期スキャンし、毎回オッズを再取得して ROI を再計算、ROI≥ゲートを買い目付きで通知
# decision-support（判断は人間・軸は不変。下記「軸ロック」「ライブ監視」参照）。セッション記録には触れない（オッズ snapshot は再取得・保存）。全レース発走で自動終了。
paddock-predict-watch --date YYYY-MM-DD          # 既定: 窓40分 / 間隔5分 / ROIゲート100% / α=本番0.2
paddock-predict-watch --date YYYY-MM-DD --once   # 1スイープのみ（cron 等）
```

- **通知ゼロを「妙味なし」と読む前にログの `⚠ 前回スイープから N 分空きました` を確認する**（#568/ADR 0072）。監視は wall-clock 基準でスリープから自動再開し、飛んだ区間を警告する（例外: プロセスがハング/死亡したまま・時計が後退した場合は警告が出ない。詳細は下記 knowledge）。この行がある日は、その間に発走したレースが未評価なので判断材料が欠けている。スリープ抑止は launchd の keep-awake（#264・開催日ごとに install が要る）に一本化されており、**蓋閉じスリープはどちらにせよ止められない**——外出中に監視を当てにするなら蓋を閉じない。詳細は [docs/knowledge/monitor-loop-sleep-resilience.md](docs/knowledge/monitor-loop-sleep-resilience.md)

### 4. 結果取得

手動で netkeiba から直接確認する（fetch-results はレース結果ページが生成されるまで使用不可。レース終了後も数分〜十数分かかることがある）:

```
https://race.netkeiba.com/race/result.html?race_id=<12桁>
```

エンコーディング: UTF-8

### 補足: netkeiba エンコーディング

- `race.netkeiba.com`（shutuba/result/odds）: **UTF-8**
- `db.netkeiba.com`（horse/result=近走）: **EUC-JP**

## 買い方ルール

> **この節を変えたら [docs/knowledge/glossary.md](docs/knowledge/glossary.md)（D07）の買い方まわりの
> 行も見直すこと。** 用語集は 印 / 軸 / 混戦の配分 / ながし / ボックス / フォーメーション /
> second source / decision-support の 8 語の要約を本節から写しているが、`CLAUDE.md` は `sources` に
> 入らない設計（ADR 0077）なので**機械検査は鳴らない**。

> 現行ルールの決定根拠・棄却記録・バックテスト履歴: [docs/specifications/betting-rule-history.md](docs/specifications/betting-rule-history.md)（ルール変更を検討する時だけ参照。予想実行時は読まなくてよい）
>
> **要件 ID と検証手段**: 下記各項の「なぜそう決めたか / どう測り直すか」は REQ 表が持つ。
> 買い方の具体は [ev-kelly-bet-selection.md](docs/specifications/ev-kelly-bet-selection.md) の **REQ-D23-001〜006**、
> 目標側（ROI ゲート・軸ロック・提示形式）は [product-goals.md](docs/knowledge/product-goals.md) の
> **REQ-D01-001 / 003 / 007**。ルールを変えるときは対応 REQ の検証手段を再実行してから。

### 予算・配分（既定）

- **¥5,000/レース**（指定が無ければこれを使う。勝手に小さく組まない）。
- **3 券種すべて使う**: ワイド ¥1,500 / 馬連 ¥1,500 / 3連複 ¥2,000（◎1頭軸ながし）。
- 相手の広さ: **3 券種とも相手 top5**（REQ-D23-002）（ワイドも top5＝実装 `build_portfolio` に統一）。かつて「ワイドは top3」と定めていたが、262R（12 開催日）の実測で top3 と top5 に有意差なし（日別 6:6・集計符号は単一外れ日依存）→ 最小変更で実装側の top5 に寄せた（ADR 0065）。
- 各点の金額は**券種予算内で 100 円単位の均等配分**（REQ-D23-003）（実装 `build_portfolio`/`distribute`）。券種予算を脚数で割り、賄える範囲で薄い脚にも同額を置く（**賄えない端数の脚は ¥0＝買わない**。脚ごとに ¥100 を必ず確保するわけではない）。券種予算は 100 円単位に切り捨てて配分するため、総賭金は券種予算以下（通常は予算ちょうど、全点に ¥100 を置けない端数時のみ下回る）。確率重み化＋脚ごと最低¥100 撤廃案は 71R で実 ROI 悪化のため棄却済み＝均等割りを維持（ADR 0046）。
- **印を打った馬は必ず買い目に絡める**（top5 まで広げる主因）。
- 上記は `predict`（記録買い目）と `predict-watch`（ライブ監視）双方の配分＝いずれも Rust `build_portfolio` の均等割り。一方 `scripts/predict-check/` のオフライン EV レポート系（Python `live_ev.py`）だけは **model 確率重み＋最低¥100 の最大剰余法**という別方式で、これが ADR 0064 の警告する second source（買い方ロジックの二重実装）。**張るのは均等割りの `build_portfolio` 買い目が正**（`predict-watch` も build_portfolio なので同じ配分）。

### 混戦判定と配分

- 判定条件: **◎の model 勝率の 0.70 倍以上の馬が ◎含め 4 頭以上** → 混戦（REQ-D23-005）。
- 混戦時の配分（¥5,000）: **3連複ボックス ¥1,500 / ワイド ¥1,000 / 馬連 ¥1,000 / 3連複◎軸ながし ¥1,500**。
- ボックス構成: 印馬（◎○▲☆、最大 5 頭）の 3 連複ボックス。組内も 100 円単位の均等配分（賄えない端数の脚は ¥0）。印馬が 2 頭以下はボックス不成立（ながし¥1,500 を ◎軸ながしに転用）。
- **相手を top5 より広げない**（バックテスト 105R で相手拡大は回収率を悪化させると確認済み）。
- **オッズ条件を混戦判定に加えない**（バックテスト 71R で baseline を上回る閾値が無いと確認済み。発動した閾値は悪化、odds≥4.0 は発動せず baseline 同値）。

### レース選択基準

> **参考ROI を自動判定として当てにしない**（ADR 0076・#602）。182 レース / 839 スイープを確定払戻で
> 精算したところ、**ROI ≥ 100% の通過は 0 件**（最大 76.8%）で、`Spearman(判定ROI, 実現ROI) = +0.002`
> ＝**どのレースが良いかを judge する力が無い**。買い目の脚は blended（市場優位）で選ぶのに EV は
> 市場情報を捨てた pure 確率で値付けするため、選ばれた脚が構造的に低く出る（閾値でなく定義の不整合）。
> **だからといって閾値は下げない**——θ を下げても実現ROIは 100% に届かない（ADR 0040 の棄却を ADR 0076 が追認）。

- **「高的中・低配当」は無価値**。断然人気は EV がマイナスになりがち。
- **−EV は見送る**という原則は変えない（REQ-D01-001）。ただし **+EV かどうかを参考ROIで判定しない**。
  張る/見送りは**手動のハンデ精査**（近走フォーム・コース/枠バイアス・距離/騎手）と
  **執行の規律**（軸ロック＋ズレ増額）で決める＝現時点で実在が確認できているエッジはこの 2 つだけ
  （[product-goals.md](docs/knowledge/product-goals.md)「エッジの所在」）。
- 参考ROI は **decision-support の材料**として読む（レース間の優劣づけには使わない）。
- 的中率ではなく **期待値で選ぶ**という考え方自体は維持する。変わったのは
  「その期待値を現行の参考ROIで測れると思わない」という点。

### 軸ロックとズレ増額（確率と買い方の分離）

> 根拠: ADR 0055（EV 層分離）の運用面 follow-up＝ADR 0060。純モデルの resolution は天井（ADR 0058/0059）で「市場より上手く当てる」路線は closed。エッジは "当てる精度" でなく **買い方（執行）の規律** に置く。

- **軸（◎と基本の買い目構造）は事前データで確定し、ブラさない**（REQ-D01-003）。近走フォーム・コース/枠バイアス・距離/騎手など事前に読める材料で軸を決めたら、直前のオッズ変動で軸をひっくり返さない。
- **発走直前オッズの用途は "ズレ増額" のみ**。軸の馬が自モデル確率より美味しくオッズがズレた（＝過小人気）ときに、**軸を含む既存の買い目の金額を上げる**（＝増額）だけに直前オッズを使う。**点数（相手）は増やさない**——相手は各券種の既定幅（3 券種とも top5）のまま。不利側にズレても軸は動かさない（レース全体の見送り＝そのレースを張らない、はあってよい）。
- **軸フリップ禁止**。直前の市場ブレンドや "妙味" で高確信の軸を別馬へ乗り換えない（実証: 直前ブレンドの軸乗り換えは誤り／朝の +EV は発走直前に剥がれる）。◎ を見直すのは、事前根拠を崩す **新情報**（発表された取消・馬場激変・重大な馬体/パドック異常など）が出た時だけ、理由を明示して行う。オッズが動いただけでは見直さない。

### ライブ監視時のコミュニケーション規律

predict-watch は **decision-support（判断材料の提示）** であって自動 go/no-go ではない（ADR 0055/0060）。ツールが出す参考 ROI は判断材料に使い、**最終判断（張る/見送り/増額）は人間が決める**。張るの一次判断は「レース選択基準」、軸・点数・相手の不変は「軸ロックとズレ増額」の通りで、監視中も同じ規律を適用する。

- **唯一の正 = 最新サイクルの判定のみ**。前サイクル・朝の +EV リストは無効化する。
- **🔶 / 🔍 を go シグナルとして読まない**（ADR 0079）。🔶（≥100%）は 839 スイープで 1 度も出ておらず、🔍（≥70%）も 3 レース・14 回だけで、**どちらもレースの優劣を示さない**。監視の起動時に同じ注記がログ先頭に出る。
- **毎回の冒頭に 1 行で現況を明示**: 軸（不変）＋「🔶増額候補あり / ⚪妙味なし（据え置き）/ ⛔レースごと見送り」。曖昧な据え置きをしない（ここで述べるのは増額＝金額アップの可否だけ）。この 3 分類は**参考ROIではなく手動精査の結論**として述べる。
- **ズレ警告必須**: 軸馬のオッズが前回から美味しくズレた（増額機会）／不利にズレた（増額見送り）を明言。レース全体の +EV↔−EV が反転したら見送り転換も明言（例:「函館10：朝+EV→直前−EVに転落、レースごと見送り」）。◎ の差し替えは「軸ロック」の**新情報**時のみ・理由明示（黙って差し替えない）。

### 表記規約（最優先）

- 買い目は **「式別 / 方式 / 軸 / 相手 / 点数 / 金額」の"そのまま買える形"**で書く（REQ-D01-007）。
- 「軸ボックス」等の曖昧語を使わない。**ながし / ボックス / フォーメーション**を正しく区別する。
  - **ながし**: 軸馬を固定し相手すべてと組み合わせ（軸必須）
  - **ボックス**: 選んだ全馬を総当たり（軸不要）
  - **フォーメーション**: 軸グループ×相手グループ（この PJ では基本不使用）
  - 例（3連複 1頭軸ながし）:「式別=3連複, 方式=ながし(軸1頭), 軸=⑩, 相手=⑬⑯⑭①⑫(5頭), 10点×300円」
- **馬券は必ず 100 円単位**（端数不可）。各レース予算ちょうどに収める。

### 軸の選び方

- **1〜2番人気の強い支持馬を"妙味"で軸から外さない**。市場は馬柱に出ない好材料（調教・厩舎の自信）を織り込む。
- contrarian な本命は「人気馬に明確な不安 ＋ 自分の本命に強い根拠」がある時だけ。

### 保険構造

- ◎1頭軸ながしだけにしない。◎が飛んでも拾えるよう **馬連 or 3連複ボックス/ながしを保険で入れる**（印が合えば軸が飛んでも拾える）。
- 馬連は ◎1頭ながし、相手は model top5 を基本（予算・配分の規定通り）。うち中穴（7〜12番人気）を 2〜3 頭は必ず含め、取りこぼしを防ぐ。

### 券種モードの使い分け

- **硬い人気馬を軸にできる時 → ワイド軸ながしを広めに**（高的中率・低変動）。
- **混戦・軸の信頼度が中程度 → 馬連流し＋3連複**（低的中率・高変動）。
- ワイドは硬い軸の堅実策として有効。軸なしの乱買いには使わない。
