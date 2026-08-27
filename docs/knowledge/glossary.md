---
status: Confirmed
kind: knowledge
doc_class: [D07]
tags: [D07]
sources:
  - docs/knowledge/product-goals.md
  - docs/knowledge/monitor-loop-sleep-resilience.md
  - docs/specifications/probability-estimation.md
  - docs/specifications/backtest.md
  - docs/specifications/ev-kelly-bet-selection.md
  - docs/specifications/betting-rule-history.md
  - docs/specifications/prediction-json.md
  - docs/specifications/prediction-search-api.md
  - docs/specifications/feature-resolution-diagnosis.md
  - docs/specifications/netkeiba-datasource.md
distilled_from_sha: "a61bf39"
updated: "2026-08-23"
---

# 用語集（ユビキタス言語）

paddock 横断の用語索引。**この文書は定義の正本ではなく、正本を指す索引**である。
各用語の定義は「正本」列の文書が持ち、ここには 1 行の要約しか置かない。定義そのものを
書き直すと third source になり、実装と仕様が二重管理になる（ADR 0064
が警告する構図と同型）。**要約と正本が食い違ったら正本が正**で、この文書を直す。

読み方の指針:

- **同じ語で別のものを指す箇所が実在する**。取り違えが実害を生む語には「⚠」を付けた。
- 買い方（D23）の運用指示の一次定義は `docs/` ではなくリポジトリルートの
  [CLAUDE.md](../../CLAUDE.md)「買い方ルール」にある。これは
  [doc-classes.md](doc-classes.md)「体系側の既知の穴」が記録済みの状態で、本文書もそこを指す。

収録と参照の基準（追加・改訂時はこれに従う。決定の記録は
ADR 0077）:

- **収録するのは複数の文書・コードにまたがって使われる語だけ**。1 つの文書の中で閉じて使われる語は
  その文書の `## 用語定義` 節に置き、ここへは持ってこない。
- **正本列は「現行値を所有している文書」を指す**。要件として固定された値（本番の α・m・γ 等）は
  REQ-ID を指し、決定の経緯だけが要るものは ADR を指す。ADR は決定時点の RO 記録なので、
  値が改訂されても動かない——**現行値の確認は REQ 表、経緯は ADR** と読み分ける。
- **値を書くときは、その値を所有する正本を必ず併記する**（要件として固定された本番値は REQ-ID、
  既定値は仕様書の該当節）。値だけを書き写して出所を書かないと、改訂されたときに気づけない。
  **分類の列挙**（transient に含まれるエラー種別など）は、それ自体が語の定義なので書く。
  逆に、正本を辿れば済む調整値（クランプの ε・リトライ回数など）は書かない。
- **`sources` に入れる基準は「その文書の本文が動いたら、ここの要約の見直しが要るか」**。
  確定知層（specifications / knowledge）も対象で、引いた ADR も入れる。上流が動けば追従コミット
  （`distilled_from_sha` の bump）が要るが、見直しの強制が狙いなのでこれは仕様。
- **本文書が `sources` に入れないもの 3 つ**: `CLAUDE.md` / ソースコード / 文書運用の規約文書
  （[README.md](README.md)・[doc-classes.md](doc-classes.md)）。**これは用語集での適用**で、
  リポジトリ全体の禁止ではない（主題そのものが対象ファイルなら入れてよい。例: `ci-pipeline.md`
  → `ci.yml`、API 系 3 本 → `openapi.json`）。`CLAUDE.md` は frontmatter を
  持たないため stale 検査の免除機構（frontmatter だけの変更の除外）が**構造的に効かず**、かつ
  大半の改訂が用語と無関係なので、入れるとすべての `CLAUDE.md` 編集に 2 コミットを強制して
  追従が儀式化する。コードの追従はそれを仕様として持つ文書側の責務、規約文書は本文書の主題では
  なく運用の参照。**`CLAUDE.md` への依存自体は残る**ので、その補償として `CLAUDE.md`
  「買い方ルール」節に人手のトリップワイヤを置いてある。
- **戻りリンク（正本 → 用語集）を置くのは `## 用語定義` 節を持つ文書だけ**。
  節が無い文書のどこに置くかは恣意的になるため。**追従対象を決めるのは `sources` であって
  戻りリンクではない**ので、戻りリンクの無い正本も stale 検査の対象になりうる。
- **対象領域は確率推定・評価・買い方・データ取得の運用**。web / API / 監視まわりの横断語
  （board・鮮度バッジ・keep-awake 等）は**まだ収録していない**（方針としての除外ではなく未着手）。

採らなかった案（ADR 0077 の却下記録の写し）:

- **定義を集約して書き下ろす**: 読みやすいが仕様書と同じ定義が 2 箇所に並び、片方だけの更新を
  機械検査が検出できない（ADR 0064 の second source と同型）。
- **`CLAUDE.md` を `sources` に入れる**: 依存としては正しく一度入れたが、全 `CLAUDE.md` 編集に
  追従コミットを強制するコストが、得られる検査価値に見合わない。
- **買い方ルールを `docs/` へ移して依存を消す**: 毎セッション自動で読まれることが実効性の source
  なので移すと「読まれる保証」を失う。移すか否かは別の ADR の仕事。
- **用語集に REQ 表を持たせる**: 用語集に要件は無い。REQ-ID は各語の正本側が持つ。

## 確率とスコア

| 用語 | 要約 | 正本 |
|---|---|---|
| `win_prob` | 勝率＝1 着以内確率。レース内合計 1.0 へ正規化 | [probability-estimation.md](../specifications/probability-estimation.md) 用語定義・ステップ 3 |
| `place_prob` | 連対率＝2 着以内確率（日本競馬の「連対」＝top-2）。合計 2.0 へ正規化（小頭数では上限クランプで下回りうる） | 同上 |
| `show_prob` | 複勝率＝3 着以内確率（日本競馬の「複勝」＝top-3）。合計 3.0 へ正規化（同上のクランプあり） | 同上 |
| ⚠ 確率のスケール | 2 系統ある。**確率推定パイプライン**（`HorseProbability` / board / analyze / `HorseProbabilitySchema`）は `[0.0, 1.0]`。**予想 JSON 系統**は百分率の表示値（例 `25.4`）で、`ingest-predictions` が変換せずそのまま保持するため `prediction_horses` テーブルと prediction-search API まで百分率が伝播する | [prediction-json.md](../specifications/prediction-json.md)（入力）/ [prediction-search-api.md](../specifications/prediction-search-api.md)（DB 列・レスポンス） |
| 単調化 | 馬ごとの累積 max で `win_prob ≤ place_prob ≤ show_prob` を保証する後処理。冪変換の後も再是正する | [probability-estimation.md](../specifications/probability-estimation.md) REQ-D22-011 / ADR 0007 |
| factor | 生スコアを構成する素性。stat 系 6 つ（`course_gate` / `horse_surface` / `horse_distance` / `jockey_surface` / `trainer_surface` / `horse_track_condition`）と scalar 系 3 つ（`recent_form` / `weight_carried` / `jockey_recent_form`）。`jockey_recent_form` は**重み 0 で実質不参加**（機構だけ残す・REQ-D22-007） | [probability-estimation.md](../specifications/probability-estimation.md) ステップ 2 |
| `raw_score` | **存在する factor のみ**の重み付き平均。欠落項は重みごと母数から除外し、0 埋め（＝全敗扱い）にしない。**この drop は `Default` の挙動で、本番は下記 impute 後の値で計算する** | [probability-estimation.md](../specifications/probability-estimation.md) ステップ 2 / ADR 0014 |
| impute（field mean 補完） | 欠落した stat factor を同レース内 present 馬の縮約後レート平均で埋める。scalar 項 3 つは対象外。`production()` は有効 | [probability-estimation.md](../specifications/probability-estimation.md) REQ-D22-010 / ADR 0057 |
| ベイズ縮約 / 擬似カウント `m` | `smoothed = (k·rate + m·prior) / (k + m)`。少データ馬の極端なレートを prior へ引き寄せる。**本番 predict は m=10**（`RECOMMENDED_SHRINKAGE_M`。backtest の既定は縮約 off） | [probability-estimation.md](../specifications/probability-estimation.md) REQ-D22-002 |
| prior（基準率） | 縮約の引き寄せ先。出走頭数 ~14 由来で win=1/14・place=2/14・show=3/14 | [probability-estimation.md](../specifications/probability-estimation.md) ステップ 2.5 |
| recency（リーセンシー重み） | 過去成績を `0.5^(days_ago/half_life)` で時間減衰させる集計。**既定は無効**（改善が確認できず、機構だけ残す） | [probability-estimation.md](../specifications/probability-estimation.md) REQ-D22-006 / ADR 0016 |
| ⚠ 冪変換 γ | 2 か所ある別物。`place_show_power` は**正規化前**の place/show に掛ける脱圧縮（本番 **2.0**）、`win_power` は**ブレンド後**の win に掛ける校正（本番 **1.25**） | [probability-estimation.md](../specifications/probability-estimation.md) REQ-D22-004 / REQ-D22-003（経緯は ADR 0047 / ADR 0042） |
| implied 確率 | 単勝オッズの逆数 `1 / odds`。控除率を含んだままの生値 | [probability-estimation.md](../specifications/probability-estimation.md) ステップ 4 |
| overround（控除率分の超過） | `Σ implied > 1.0` の超過分。合計 1.0 へ正規化して除いたものが市場確率 | 同上 |
| `blended` | `blended = α·model + (1−α)·market`。**α はモデル重み**で、α=1.0 が純モデル・α=0.0 が市場のみ。**本番既定 α=0.2**（`RECOMMENDED_MARKET_BLEND_ALPHA`。オッズが無いレースはモデルのみへ自動フォールバック） | [probability-estimation.md](../specifications/probability-estimation.md) REQ-D22-001（採用の経緯は ADR 0034） |
| ⚠ 純モデル確率（pure） | ブレンド前のモデル確率。**順位付けは `blended`、EV 計算は pure** という層分離を守る | ADR 0055 / [product-goals.md](product-goals.md) REQ-D01-002 |
| resolution（判別力） | 「どの馬が勝つか」を見分けるランクの強さ。AUC・top1 で測る。純モデルは市場に劣ることが確定済み | [feature-resolution-diagnosis.md](../specifications/feature-resolution-diagnosis.md) |
| calibration（較正） | 予測確率と実測頻度の一致度。Brier・LogLoss・reliability 曲線で測る。**resolution とは別軸**（較正しても判別力は生まれない） | 同上 / [backtest.md](../specifications/backtest.md) |

## 評価・バックテスト

| 用語 | 要約 | 正本 |
|---|---|---|
| 評価レース | 期間内かつ `source='pdf'`・`finishing_position` を持つ確定済みレース | [backtest.md](../specifications/backtest.md) 用語定義 |
| as-of 統計 | レース日 D に対し `races.date < D` の成績だけで集計した統計（リーク防止） | 同上 |
| walk-forward | 各評価レースで「その時点までに得られた情報のみ」を使う時系列評価方式 | 同上 |
| トップ選好馬 | `win_prob` が最大の馬。単勝の本命として扱い、連対・複勝の的中率もこの馬で測る | 同上 |
| Brier (win) | `mean((win_prob − y)²)`、y=1 if 1 着。全馬エントリ単位。**小さいほど良い** | [backtest.md](../specifications/backtest.md) ステップ 4: 指標集計 |
| LogLoss (win) | `−mean(y·ln p + (1−y)·ln(1−p))`。`p` はクランプして `ln(0)` を回避する | 同上 |
| reliability 曲線 | `win_prob` を等幅 10 ビンに分け、ビンごとの「平均予測確率 vs 実測勝率」を並べたもの。平均予測 > 実測なら過大評価 | 同上 |
| 想定回収率 | `Σ payout / Σ stake`。各レース 100 円をトップ選好馬の単勝に賭けた仮定値。**実際の買い方（3 券種）とは別物** | 同上 |
| `ev`（期待値） | `probability × odds`。1.0 を超えると理論的にプラス期待値 | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) 用語定義 |
| ROI（レース単位） | `Σ_i(賭金_i × 的中確率_i × 払戻倍率_i) / 総賭金`。買い目全体で見た期待回収率 | [CLAUDE.md](../../CLAUDE.md)「3. EV 判定 → 買い目決定」 |
| ⚠ ROI ゲート | 元は「**ROI ≥ 100% のレースだけ張る**」という判定基準（100% が損益分岐）。**現在は参考 ROI をこの判定に使わない**——182R 実測でゲート通過 0 件・判定 ROI と実現 ROI は無情報だったため（張る/見送りは手動のハンデ精査と執行の規律で決める）。**それでも閾値は下げない**（下げる＝−EV を承知で買う。θ を下げても実現 ROI は 100% に届かない）。`predict-watch` は 🔶 / 🔍 のマークを残したまま、起動時に到達不能である旨を注記する | [product-goals.md](product-goals.md) REQ-D01-001・「ゲートの現況」/ ADR 0040（閾値）/ ADR 0076（現況）/ ADR 0079（表示と運用記述） |
| フェア ROI | JRA 控除率（ワイド・馬連 22.5% / 3 連複 25%）由来の期待値上限 ≈ 75〜77.5%。エッジが無ければ ROI はこの近辺に落ちる | [betting-rule-history.md](../specifications/betting-rule-history.md) ⑤ |
| `ev_threshold` / `trifecta_ev_threshold` | 推奨候補に入れる EV の閾値（既定 1.0 / 三連単のみ 2.0）。判定は strict なので**閾値ちょうどは除外**される（用語定義の「これ以上の EV」は不正確で、「3. EV フィルタ」が正） | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md)「3. EV フィルタ」（判定）/ 用語定義（既定値） |
| ⚠ `kelly_fraction` / `kelly_cap` | 総資金に対する賭け割合と、その上限（既定 0.25）。**本番の賭け額配分には使わない**——用途は薄い買い目を落とす `min_kelly` の curation だけで、配分は均等割り | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) 用語定義（既定値）/ REQ-D23-004（用途制限） |

## 買い方・運用ルール

| 用語 | 要約 | 正本 |
|---|---|---|
| 印 | 予想の格付け記号。**運用で打つのは ◎○▲☆**（◎が本命＝軸。印を打った馬は必ず買い目に絡める＝相手を top5 まで広げる主因）だが、**データモデルは △・注 を含む 6 種**（`honmei`/`taikou`/`tanana`/`renge`/`hoshi`/`chui`） | [CLAUDE.md](../../CLAUDE.md)「予算・配分（既定）」「混戦判定と配分」 / [prediction-json.md](../specifications/prediction-json.md)（6 種） |
| 軸 | ◎に据えて買い目の中心に固定する馬 | [CLAUDE.md](../../CLAUDE.md)「軸ロックとズレ増額（確率と買い方の分離）」「軸の選び方」 |
| 軸ロック | **軸と基本の買い目構造（軸・相手・混戦判定）を事前データで確定し、直前のオッズ変動でひっくり返さない**規律。見直すのは取消・馬場激変などの新情報が出たときだけ。`predict-watch` の実装では**その日の初回スイープ**で確定する | ADR 0060（規律）/ ADR 0078（実装）/ [product-goals.md](product-goals.md) REQ-D01-003 / [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) REQ-D23-007 |
| ズレ増額 | 軸が自モデル確率より過小人気にズレたとき、**既存の買い目の金額だけを上げる**こと。点数（相手）は増やさない | 同上 |
| 混戦 | ◎の model 勝率の **0.70 倍以上の馬が ◎含め 4 頭以上**いる状態（判定条件）。この状態で 3 連複ボックスを含む別配分に切り替える | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) REQ-D23-005（判定）/ [CLAUDE.md](../../CLAUDE.md)「混戦判定と配分」（配分） |
| 相手 top5 | 3 券種とも model 確率上位 5 頭を相手に取る既定幅。**広げない**（上限側を直接測ったのは 3 連複のみ＝ADR 0030。既定の 5 頭自体は ADR 0019 が置いた設計値） | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) REQ-D23-002（status: Tentative） |
| ながし | 軸馬を固定し、相手すべてと組み合わせる方式（軸が必須） | [CLAUDE.md](../../CLAUDE.md)「表記規約（最優先）」 |
| ボックス | 選んだ全馬を総当たりする方式（軸は不要） | 同上 |
| フォーメーション | 軸グループ×相手グループで組む方式。**このプロジェクトでは基本不使用** | 同上 |
| 均等割り配分 | 券種予算を脚数で割った 100 円単位の等額配分。賄えない端数の脚は ¥0。実装 `build_portfolio` が正。**券種予算の既定は円建て（馬連 1500 / ワイド 1500 / 3連複 2000）** で、¥5,000・相手 5 頭なら予算ちょうど張り切る | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) REQ-D23-003（経緯は ADR 0080） |
| ⚠ second source（買い方の二重実装） | 張るのは Rust `build_portfolio`（均等割り）の買い目。`scripts/predict-check/live_ev.py` は確率重み＋最低 ¥100 の別方式で、オフライン EV レポート専用 | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) REQ-D23-006 / [CLAUDE.md](../../CLAUDE.md)「予算・配分（既定）」 |

## 実行・運用

| 用語 | 要約 | 正本 |
|---|---|---|
| ⚠ degraded | **単複オッズの取得失敗だけ**が該当する状態。オッズ保存をまるごとスキップして **exit 3** を返す（出馬表・近走は保存済み） | ADR 0049 / [netkeiba-datasource.md](../specifications/netkeiba-datasource.md) |
| ⚠ 対象外スキップ | 障害レース等の取り込み対象外。DB 無変更で理由を stdout に出し **exit 0**。degraded（3）ともハード失敗（1）とも別 | [netkeiba-datasource.md](../specifications/netkeiba-datasource.md)（終了コード）/ ADR 0075（経緯） |
| best-effort | 失敗しても全体を巻き添えにしない経路（オッズ**未発売**・近走取り込み・組合せ券種オッズ）。**exit 0 のまま**なので件数はログで見る | [netkeiba-datasource.md](../specifications/netkeiba-datasource.md) |
| 未発売番兵 | netkeiba が未発売・該当なしの組み合わせに入れる**券種ごとの**固定値（ワイド `9999.9` / 馬連・馬単・三連複 `99999.9` / 三連単 `999999.9`。単勝・複勝には無い）。**払戻倍率ではない**ので EV に入れない——入ると 1 点で EV が 3 桁になり参考 ROI が跳ねる。判定は**券種スコープの特定値除外**（上限方式は正当な高配当を殺すので採らない。同じ値でも券種が違えば正当——三連複の `9999.9` は配当として実在し得る） | [netkeiba-datasource.md](../specifications/netkeiba-datasource.md)「未発売の番兵値」/ ADR 0086 / ADR 0088 |
| 未発売観測 | 「この券種は netkeiba 上で未発売だと**確認できた**」という記録（`race_odds_unpriced_observations`）。read-through の cache-hit 判定で欠落券種から差し引き、券種まるごと未発売の時間帯に再スクレイプが止まらなくなるのを防ぐ。判定は「取得成功なのに priced が 0 件か」（番兵の有無では見ない）。**取得失敗は観測しない**——「分からない」を「売っていない」にすると #294 の自己修復が鈍る。TTL 15 分＝発売開始に気づくまでの最大遅れ | [netkeiba-datasource.md](../specifications/netkeiba-datasource.md)「保存したオッズの読み出しと read-through」/ ADR 0089 |
| transient | リトライ対象の一過性障害（`Timeout` / `Io` / `ConnectionFailed` / `HostNotFound` / `Protocol` / 5xx）。回数とバックオフは正本 | [netkeiba-datasource.md](../specifications/netkeiba-datasource.md)「transient リトライと degraded」/ ADR 0049（経緯） |
| decision-support | ツールは判断材料を出すだけで、**張る / 見送り / 増額の最終判断は人間**という位置づけ。`predict-watch` の設計原則 | ADR 0055 / ADR 0060 / [CLAUDE.md](../../CLAUDE.md)「ライブ監視時のコミュニケーション規律」 |
| ⚠ predict-watch の 3 つの閾値 | `--roi-gate`（🔶 の**表示**・買う閾値）/ `--notify-gate`（🔍 の**表示**・検証候補帯の下端）/ `--notify-roi`（**macOS 通知の発火**・既定＝roi-gate）。**前 2 つは表示だけでベルは鳴らない**——名前に反して通知を鳴らすのは `--notify-roi` だけ。鳴っても go シグナルではない（ADR 0079） | [README.md](../../README.md)「発走直前の EV/ROI 監視」/ [monitor-loop-sleep-resilience.md](monitor-loop-sleep-resilience.md) 規律 4（#584 / #345） |

## 文書運用の用語

蒸留モデルそのものの用語（3 層・`sources`・`distilled_from_sha`・stale・`doc_class`・REQ-ID）は
[README.md](README.md) と [doc-classes.md](doc-classes.md) が正本なので、ここでは重複させない。

---

## 決定ログ

<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->

### ADR 0077: 用語集は正本を指す索引に徹し、`sources` は確定知層まで許すが `CLAUDE.md` は入れない (2026-08-12) — 承認済み

#### ステータス

承認済み（本 PR で実装）。対象 Issue: [#598](https://github.com/taito-station/paddock/issues/598)。
関連: ADR 0073（文書クラス体系と ADR の一次資料層統合）。

#### コンテキスト

ADR 0073 が置いた文書クラス D01〜D24 のうち、**D07（用語集・ドメインモデル定義書）だけが実体のある
充足ギャップ**だった（D05 / D16 / D18 は「近い文書が既にある」「他が担っている」で当面閉じている）。
実害は用語の散在で、`## 用語定義` 節を持つ文書が 3 本あり、それぞれが 3〜5 語を独自に定義しているだけで
横断の正本が無い。`raw_score` / `blended` / 縮約 m / 冪変換 γ は仕様書の本文中に手順として書かれ、
`軸ロック` / `混戦` / `ながし・ボックス` に至っては一次定義が `CLAUDE.md`（`docs/` 配下ではない）にある。

とくに危ないのは**同じ名前で別のものを指す語**で、実際に次の取り違えが起こりうる状態だった。

- `win_prob` は確率推定パイプラインでは `[0, 1]`、予想 JSON 系統では百分率の表示値。
  `ingest-predictions` が変換せず保存するため `prediction_horses` と prediction-search API まで
  百分率が伝播するが、**この伝播をどの仕様書も書いていなかった**。
- `degraded`（単複オッズ取得失敗のみ・exit 3）と対象外スキップ（exit 0）は別物。
- `kelly_fraction` は本番の賭け額配分に使わない（用途は `min_kelly` の curation だけ）。

用語集を作るとき、書き方には 2 つの方向がある。**定義を集約して書き下ろす**か、**定義の所在を指す
索引にする**か。前者は読みやすいが、同じ定義が 2 箇所に生まれ、片方だけ更新されても機械検査は
検出できない（ADR 0064 が警告した二重実装と同型）。

もう 1 つ、`sources` の扱いに未定義の領域があった。[docs/knowledge/README.md](../knowledge/README.md)
の frontmatter 規約は `sources` を「由来（ADR / qa / docs-original のパス）」と定義していたが、実際には
確定知層の仕様書を `sources` に取る文書が既に 3 本ある（`live-freshness-calibration.md` /
`analyze-search-and-state.md` / `race-card-display-metadata.md`）。用語集は性質上、**定義の所在＝確定知層**
を由来に取らざるを得ず、規約と実態の乖離を埋める必要があった。

#### 決定

1. **用語集（[docs/knowledge/glossary.md](../knowledge/glossary.md)）は「定義の正本を指す索引」に徹する。**
   各語に置くのは 1 行の要約と正本へのリンクだけで、定義を書き下ろさない。**要約と正本が食い違ったら
   正本が正**とし、用語集を直す。値（本番の α・m・γ、混戦の閾値など）は書いてよいが、
   **その値を所有する正本（要件として固定されたものは REQ-ID、既定値は仕様書の節）を必ず併記する**。

2. **`sources` に確定知層（`docs/specifications/` / `docs/knowledge/`）を取ってよい。**
   判定基準は「**その文書の本文が動いたら、こちらの要約の見直しが要る関係にあるか**」。
   `docs/knowledge/*.md` を `sources` に取るのは本 ADR が初例（`docs/specifications/*.md` は先例あり）。
   README の frontmatter 規約もこれに合わせて明文化する。

3. **`CLAUDE.md` は `sources` に入れない。** 正本列で指すに留める。理由は 2 つ:
   - `CLAUDE.md` は frontmatter を持たないため、`scripts/check-doc-classes.py` の
     `is_metadata_only_change`（frontmatter だけの変更を stale から免除する機構）が**構造的に効かない**。
   - 多目的な運用指示書で、大半の改訂（DB 運用・予想ワークフロー・取得手順）は用語と無関係。
     入れると**すべての `CLAUDE.md` 編集に「本文＋sha 追従」の 2 コミットを強制する**ことになり、
     追従が儀式化して stale の意味が失われる。

   代償として、`CLAUDE.md` の買い方ルールが変わっても用語集は検査されない。**`CLAUDE.md`
   「買い方ルール」節に人手のトリップワイヤ（1 行の注意書き）を置いて補償する**。
   **本文書（用語集）では**同じ理由でソースコードと文書運用の規約文書（`README.md` / `doc-classes.md`）も
   `sources` に入れない。これは用語集に限った適用で、リポジトリ全体の禁止ではない——`ci-pipeline.md` は
   `.github/workflows/ci.yml` を、API 系 3 本は `docs/api/openapi.json` を `sources` に取っており、
   **その文書の主題そのものが対象ファイルであるなら入れてよい**。

#### 理由

**索引に徹するのは、二重定義を機械検査が検出できないから。** 同じ定義が仕様書と用語集の 2 箇所にあると、
片方だけ更新されても stale 検査は鳴らない（stale が見るのは `sources` の sha だけで、本文の一致は見ない）。
索引なら食い違いは「正本が正」で一意に解決でき、要約がズレても正本を併記してあるので突合できる。

**`sources` の判定を「本文が動いたら見直しが要るか」に置くのは、パスの種類で決めると実態に合わないから。**
確定知層を由来から外すと、定義の所在を指す索引は `sources` を持てず stale 検査の外に出る。
逆に `CLAUDE.md` のように免除機構が構造的に効かないファイルを入れると、無関係な改訂まで追従を強制して
検査が儀式になる。**検査が鳴ったときに実際に見直す必要があるか**が唯一の実用的な基準になる。

#### 却下した代替案

- **定義を用語集に集約して書き下ろす**: 読みやすさは上がるが、仕様書と用語集に同じ定義が並び、
  片方だけの更新を機械検査が検出できない。ADR 0064 が警告する second source と同型。
  本 PR のセルフレビューでも、要約に書いた列挙が正本とズレる事故が実際に起きている
  （factor の列挙から重み 0 の `jockey_recent_form` が落ちた）。索引に徹する方針でも
  この種のズレは起こるが、**正本を併記していれば突合できる**。
- **`CLAUDE.md` を `sources` に入れる**: 依存関係としては正しく、一度は入れた。しかし上記の
  コスト（全 `CLAUDE.md` 編集への 2 コミット強制）が、得られる検査価値
  （用語に関係する改訂は `CLAUDE.md` 全体の改訂のごく一部）に見合わない。
- **`CLAUDE.md` の買い方ルールを `docs/` へ移して依存を消す**: 依存は消えるが、買い方ルールは
  **毎セッション自動で読まれることが実効性の source**で、移すと「読まれる保証」を失う。
  この論点は [doc-classes.md](../knowledge/doc-classes.md)「体系側の既知の穴」が記録済みで、
  移すかどうかは本 ADR の範囲外（別の ADR で決める）。
- **用語集に REQ 表を持たせる**: 用語集に要件は無い。REQ-ID は各語の正本側（D01 / D22 / D23）が持つ。

#### 影響

- D07 の充足ギャップが解消し、`check-doc-classes.py` の警告は 4 件 → 3 件（D05 / D16 / D18）になる
  （いずれも決定時点の実測。以降の件数は checker の出力を見る）。
  ただし **D07 の名称の後半（ドメインモデル定義書）は未充足のまま**で、「現行 1」になったことで
  充足ギャップの warning からは見えなくなる。この穴は `doc-classes.md`「体系側の既知の穴」に残す。
- `sources` に可変な確定知層（決定時点で 9 本）を取るため、それらの本文が動くたびに用語集の
  `distilled_from_sha` 追従が要る。**これはコストではなく仕様**（見直しの強制が狙い）。
- 用語集の本文リンクと節名参照は機械検査の対象外。REQ 表を持たない文書なので
  `check-doc-classes.py` のリンク実在検査が走らない。検査の空白は
  [#604](https://github.com/taito-station/paddock/issues/604) で追跡する。

### ADR 0079: 参考ROIのゲート表示は残し「到達不能」を明示する（閾値は動かさない・採用） (2026-08-12) — 採用

#### ステータス

採用（`predict-watch` の起動時注記と CLAUDE.md の運用記述を変更する。
**マーク判定 `mark_for` と `--roi-gate` / `--notify-gate` の既定値は変更しない**）

#### コンテキスト

ADR 0076（#571）が、EV 層分離後の参考ROIは**レースを選別するゲート指標として機能していない**と
182 レース / 839 スイープの実測で確定した。

- **ROI ≥ 100%（🔶）の通過 0 件**。判定ROIの最大は 76.8%、平均 23.2%
- `Spearman(判定ROI, 実現ROI) = +0.002`＝**実現ROI に対する選別力なし**（逆予測ですらなく無情報）
- 判定ROI ÷ 市場整合ROI(=1−控除率 77.0%) = 0.30。買い目の脚は blended（市場優位）で選ぶのに
  EV は市場情報を捨てた pure 確率で値付けするため、選ばれた脚が構造的に低く出る

にもかかわらず CLAUDE.md「レース選択基準」は「**ROI ≥ 100% のレースだけ張る**」と書いたままで、
**満たしようのない運用指示**が残っていた。監視ログにも 🔶 が出る前提の表記があり、
「今日は妙味が無かった」と「そもそも出ない指標を待っていた」が区別できない状態だった（#602）。

なお 🔍（`--notify-gate` 既定 70%）は 839 スイープ中 **14 回・3 レース**で発火しており、こちらは生きている。

#### 決定

1. **🔶 / 🔍 のマーク表示は残す。** `mark_for` の判定も既定閾値も変えない。
2. **`predict-watch` の起動時に 1 回だけ**、参考ROIの読み方を注記する（`gate_caveat_lines`）。
   - 🔶 は 182R / 839 スイープの実測で到達 0 件・実現ROI への選別力なし（ADR 0076）
   - 🔍 は結果照合の目印で張り推奨ではない（`notify_gate == roi_gate` で帯が空なら本行は出さない）
   - 張る/見送りは参考ROIで決めず、手動のハンデ精査と執行の規律（軸ロック＋ズレ増額）で決める
3. **CLAUDE.md を実測に合わせる。**「ROI ≥ 100% のレースだけ張る」→「参考ROIを自動判定として
   当てにしない」。「−EV は見送る」原則は残すが、**+EV かどうかを参考ROIで判定しない**と明記する。
   ライブ監視の規律にも「🔶 / 🔍 を go シグナルとして読まない」を足す。
4. **閾値は下げない。** ADR 0040（ゲート閾値引き下げの棄却）は ADR 0076 でも支持されている
   ——θ=30% まで下げても実現ROIは 80.7% で 100% に届かない。

#### 理由

- **「出ない」ことと「出す価値が無い」ことは別。** 🔍 帯は実際に発火し、結果照合（この判定の日は
  どうだったか）の目印として機能している。マークごと消すと、後から検証する手掛かりまで失う。
- **注記は起動時 1 回に限る。** スイープごとに出すと 1 開催日で 80 回を超え、肝心の判定行が埋もれる
  ——#584 が問題にしている「判定がログに埋もれる」を自分で再生産することになる。
- **閾値を動かすのは実測に反する。** ADR 0076 は「実現 100% に相当する判定ROI 値が存在しない」
  「Spearman ≈ 0 で順序づけの力が無い」と測った。**意味のある閾値が無いと分かっているのに
  別の値へ動かせば、動かした先の値に意味があるかのような誤解を生む**。閾値は据え置き、
  「この指標では選べない」ことを言葉で明示するほうが誠実。
- CLAUDE.md は毎セッション読まれる運用指示の正本なので、**そこが実測と食い違っているのが最大の実害**。

#### 却下した案

- **🔶 を廃止して参考ROIだけ出す** — ADR 0055 決定 4（自動の張る/見送り判定を出さない）の字義には
  最も忠実。だが `--roi-gate` フラグの意味が宙に浮き、CLI 互換性の整理が要る割に、
  誤読の防止は注記でも達成できる。🔍 帯を残す以上マーク体系自体は残るので、片方だけ消す非対称も避けたい。
- **既定 `--roi-gate` を実測に合わせて下げる**（例 70%・30%）— 「その閾値なら選べる」という含意が生じるが、
  Spearman ≈ 0 なのでどの閾値にも選別力は無い。ADR 0040 の棄却理由（緩めると −EV を買う）にも反する。
- **文言だけ直してコードは触らない** — CLAUDE.md は直っても、監視ログを見る人には
  「🔶 が出ることがある」と見えたまま。運用の実害（出ない指標を待つ）が残る。

#### 影響

- `src/apps/predict-watch/src/watch.rs`: `gate_caveat_lines` / `print_gate_caveat` を追加し、
  `run()` が `run_monitor_loop` の直前で 1 回呼ぶ。**マーク判定・閾値・スイープ出力は不変**。
- `CLAUDE.md`: 「予想ワークフロー 3. EV 判定」「レース選択基準」「ライブ監視時のコミュニケーション規律」。
- [product-goals.md](../knowledge/product-goals.md) REQ-D01-001 は #571 で既に実測を反映済みなので、
  出典に本 ADR を足すに留める。
- **#584（ROIゲート通過を通知で人に届ける）の前提が変わる。** 現行閾値では通知が 1 度も鳴らないため、
  #584 に着手するなら「何を通知するか」から決め直す必要がある。

#### 検証

- `cargo test -p predict-watch` の `gate_caveat_states_unreachable_with_evidence` /
  `gate_caveat_omits_notify_line_when_band_is_empty`（文言が実測と ADR 番号を含むこと、
  🔍 帯が空のときは案内しないこと）。
- 既存の `mark_for` / `resolve_notify_gate` テストが**不変で通ること**＝閾値・判定を変えていない担保。

#### 関連

- 出自: #602（ROIゲートの運用記述を ADR 0076 の実測に合わせる）
- 前提: ADR 0076（参考ROIはゲート指標として使えない）/ ADR 0055 決定 4（decision-support 化）/ ADR 0060（軸ロック）
- 関連: ADR 0040（ゲート閾値引き下げの棄却）/ #345（🔍 検証候補帯の導入）/ #584（通知・本 ADR で前提が変わる）
