---
status: Confirmed
kind: knowledge
doc_class: [D07]
tags: [D07]
sources:
  - CLAUDE.md
  - docs/knowledge/product-goals.md
  - docs/specifications/probability-estimation.md
  - docs/specifications/backtest.md
  - docs/specifications/ev-kelly-bet-selection.md
  - docs/specifications/betting-rule-history.md
  - docs/specifications/prediction-json.md
  - docs/specifications/feature-resolution-diagnosis.md
  - docs/specifications/netkeiba-datasource.md
  - docs/original-docs/0007-probability-monotonicity-jockey.md
  - docs/original-docs/0014-none-baseline-exclusion.md
  - docs/original-docs/0016-shrinkage-and-recency.md
  - docs/original-docs/0019-portfolio-generator.md
  - docs/original-docs/0030-konsen-trio-partner-width-rejected.md
  - docs/original-docs/0034-alpha-retune-recency-rejected.md
  - docs/original-docs/0040-ev-gate-threshold-lowering-rejected.md
  - docs/original-docs/0042-win-power-calibration-adopted.md
  - docs/original-docs/0047-place-show-power-decompression-adopted.md
  - docs/original-docs/0049-netkeiba-odds-transient-retry-and-degraded-exit.md
  - docs/original-docs/0055-ev-layer-separation-circular-break.md
  - docs/original-docs/0057-impute-missing-factors-field-mean.md
  - docs/original-docs/0060-betting-axis-lock-preclose-topup.md
  - docs/original-docs/0064-live-ev-buy-view.md
  - docs/original-docs/0075-unsupported-race-skip-exit-zero.md
distilled_from_sha: "c6fce5b"
updated: "2026-08-12"
---

# 用語集（ユビキタス言語）

paddock 横断の用語索引。**この文書は定義の正本ではなく、正本を指す索引**である。
各用語の定義は「正本」列の文書が持ち、ここには 1 行の要約しか置かない。定義そのものを
書き直すと third source になり、実装と仕様が二重管理になる（[ADR 0064](../original-docs/0064-live-ev-buy-view.md)
が警告する構図と同型）。**要約と正本が食い違ったら正本が正**で、この文書を直す。

読み方の指針:

- **同じ語で別のものを指す箇所が実在する**。取り違えが実害を生む語には「⚠」を付けた。
- 買い方（D23）の運用指示の一次定義は `docs/` ではなくリポジトリルートの
  [CLAUDE.md](../../CLAUDE.md)「買い方ルール」にある。これは
  [doc-classes.md](doc-classes.md)「体系側の既知の穴」が記録済みの状態で、本文書もそこを指す。

収録と参照の基準（追加・改訂時はこれに従う）:

- **収録するのは複数の文書・コードにまたがって使われる語だけ**。1 つの文書の中で閉じて使われる語は
  その文書の `## 用語定義` 節に置き、ここへは持ってこない。
- **正本列は「現行値を所有している文書」を指す**。要件として固定された値（本番の α・m・γ 等）は
  REQ-ID を指し、決定の経緯だけが要るものは ADR を指す。ADR は決定時点の RO 記録なので、
  値が改訂されても動かない——**現行値の確認は REQ 表、経緯は ADR** と読み分ける。
- **`sources` には要約の出所になった文書を全部入れる**（`CLAUDE.md` を含む）。上流が動いたら
  stale 検査で本文書の見直しが強制されるのが狙いで、追従コストは `distilled_from_sha` の bump。
- **戻りリンク（正本 → 用語集）を置くのは `## 用語定義` 節を持つ文書だけ**。節を持たない文書に
  戻りリンクを撒くと、用語と無関係な編集まで本文書の追従対象に見えてしまう。

## 確率とスコア

| 用語 | 要約 | 正本 |
|---|---|---|
| `win_prob` | 勝率＝1 着以内確率。レース内合計 1.0 へ正規化 | [probability-estimation.md](../specifications/probability-estimation.md) 用語定義・ステップ 3 |
| `place_prob` | 連対率＝2 着以内確率（日本競馬の「連対」＝top-2）。合計 2.0 へ正規化 | 同上 |
| `show_prob` | 複勝率＝3 着以内確率（日本競馬の「複勝」＝top-3）。合計 3.0 へ正規化 | 同上 |
| ⚠ 確率のスケール | 仕様・DB・API はいずれも `[0.0, 1.0]`。**`ingest-predictions` の入力 JSON だけは百分率の表示値**（`src/apps/ingest-predictions/src/input.rs`） | [prediction-json.md](../specifications/prediction-json.md) |
| 単調化 | 馬ごとの累積 max で `win_prob ≤ place_prob ≤ show_prob` を保証する後処理。冪変換の後も再是正する | [probability-estimation.md](../specifications/probability-estimation.md) REQ-D22-011 / [ADR 0007](../original-docs/0007-probability-monotonicity-jockey.md) |
| factor | 生スコアを構成する素性。stat 系 6 つ（`course_gate` / `horse_surface` / `horse_distance` / `jockey_surface` / `trainer_surface` / `horse_track_condition`）と scalar 系 3 つ（`recent_form` / `weight_carried` / `jockey_recent_form`）。`jockey_recent_form` は**重み 0 で実質不参加**（機構だけ残す・REQ-D22-007） | [probability-estimation.md](../specifications/probability-estimation.md) ステップ 2 |
| `raw_score` | **存在する factor のみ**の重み付き平均。欠落項は重みごと母数から除外し、0 埋め（＝全敗扱い）にしない | [ADR 0014](../original-docs/0014-none-baseline-exclusion.md) |
| impute（field mean 補完） | 欠落した stat factor を同レース内 present 馬の縮約後レート平均で埋める。scalar 項 3 つは対象外。`production()` は有効 | [probability-estimation.md](../specifications/probability-estimation.md) REQ-D22-010 / [ADR 0057](../original-docs/0057-impute-missing-factors-field-mean.md) |
| ベイズ縮約 / 擬似カウント `m` | `smoothed = (k·rate + m·prior) / (k + m)`。少データ馬の極端なレートを prior へ引き寄せる。**本番 predict は m=10**（`RECOMMENDED_SHRINKAGE_M`。backtest の既定は縮約 off） | [probability-estimation.md](../specifications/probability-estimation.md) REQ-D22-002 |
| prior（基準率） | 縮約の引き寄せ先。出走頭数 ~14 由来で win=1/14・place=2/14・show=3/14 | [probability-estimation.md](../specifications/probability-estimation.md) ステップ 2.5 |
| recency（リーセンシー重み） | 過去成績を `0.5^(days_ago/half_life)` で時間減衰させる集計。**既定は無効**（改善が確認できず、機構だけ残す） | [probability-estimation.md](../specifications/probability-estimation.md) REQ-D22-006 / [ADR 0016](../original-docs/0016-shrinkage-and-recency.md) |
| ⚠ 冪変換 γ | 2 か所ある別物。`place_show_power` は**正規化前**の place/show に掛ける脱圧縮（本番 **2.0**）、`win_power` は**ブレンド後**の win に掛ける校正（本番 **1.25**） | [probability-estimation.md](../specifications/probability-estimation.md) REQ-D22-004 / REQ-D22-003 |
| implied 確率 | 単勝オッズの逆数 `1 / odds`。控除率を含んだままの生値 | [probability-estimation.md](../specifications/probability-estimation.md) ステップ 4 |
| overround（控除率分の超過） | `Σ implied > 1.0` の超過分。合計 1.0 へ正規化して除いたものが市場確率 | 同上 |
| `blended` | `blended = α·model + (1−α)·market`。**α はモデル重み**で、α=1.0 が純モデル・α=0.0 が市場のみ。**本番既定 α=0.2**（`RECOMMENDED_MARKET_BLEND_ALPHA`。オッズが無いレースはモデルのみへ自動フォールバック） | [probability-estimation.md](../specifications/probability-estimation.md) REQ-D22-001 |
| ⚠ 純モデル確率（pure） | ブレンド前のモデル確率。**順位付けは `blended`、EV 計算は pure** という層分離を守る | [ADR 0055](../original-docs/0055-ev-layer-separation-circular-break.md) / [product-goals.md](product-goals.md) REQ-D01-002 |
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
| LogLoss (win) | `−mean(y·ln p + (1−y)·ln(1−p))`。`p` は `[ε, 1−ε]` にクランプ（ε=1e-15） | 同上 |
| reliability 曲線 | `win_prob` を等幅 10 ビンに分け、ビンごとの「平均予測確率 vs 実測勝率」を並べたもの。平均予測 > 実測なら過大評価 | 同上 |
| 想定回収率 | `Σ payout / Σ stake`。各レース 100 円をトップ選好馬の単勝に賭けた仮定値。**実際の買い方（3 券種）とは別物** | 同上 |
| `ev`（期待値） | `probability × odds`。1.0 を超えると理論的にプラス期待値 | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) 用語定義 |
| ROI（レース単位） | `Σ_i(賭金_i × 的中確率_i × 払戻倍率_i) / 総賭金`。買い目全体で見た期待回収率 | [CLAUDE.md](../../CLAUDE.md)「3. EV 判定 → 買い目決定」 |
| ROI ゲート | **ROI ≥ 100% のレースだけ張る**という判定基準。100% が損益分岐で、下げる＝−EV を承知で買う | [product-goals.md](product-goals.md) REQ-D01-001 / [ADR 0040](../original-docs/0040-ev-gate-threshold-lowering-rejected.md) |
| フェア ROI | JRA 控除率（ワイド・馬連 22.5% / 3 連複 25%）由来の期待値上限 ≈ 75〜77.5%。エッジが無ければ ROI はこの近辺に落ちる | [betting-rule-history.md](../specifications/betting-rule-history.md) ⑤ |
| `ev_threshold` / `trifecta_ev_threshold` | 推奨候補に入れる EV の下限（既定 1.0 / 三連単のみ 2.0） | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) 用語定義 |
| ⚠ `kelly_fraction` / `kelly_cap` | 総資金に対する賭け割合と、その上限（既定 0.25）。**本番の賭け額配分には使わない**——用途は薄い買い目を落とす `min_kelly` の curation だけで、配分は均等割り | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) REQ-D23-004 |

## 買い方・運用ルール

| 用語 | 要約 | 正本 |
|---|---|---|
| 印（◎○▲☆） | 予想の格付け記号。◎が本命＝軸。印を打った馬は必ず買い目に絡める（相手を top5 まで広げる主因） | [CLAUDE.md](../../CLAUDE.md)「予算・配分（既定）」「混戦判定と配分」 |
| 軸 | ◎に据えて買い目の中心に固定する馬 | 同上 |
| 軸ロック | **軸と基本の買い目構造を事前データで確定し、直前のオッズ変動でひっくり返さない**規律。見直すのは取消・馬場激変などの新情報が出たときだけ | [ADR 0060](../original-docs/0060-betting-axis-lock-preclose-topup.md) / [product-goals.md](product-goals.md) REQ-D01-003 |
| ズレ増額 | 軸が自モデル確率より過小人気にズレたとき、**既存の買い目の金額だけを上げる**こと。点数（相手）は増やさない | 同上 |
| 混戦 | ◎の model 勝率の **0.70 倍以上の馬が ◎含め 4 頭以上**いる状態。3 連複ボックスを含む別配分に切り替える | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) REQ-D23-005 |
| 相手 top5 | 3 券種とも model 確率上位 5 頭を相手に取る既定幅。**広げない**（上限側を直接測ったのは 3 連複のみ＝[ADR 0030](../original-docs/0030-konsen-trio-partner-width-rejected.md)。既定の 5 頭自体は [ADR 0019](../original-docs/0019-portfolio-generator.md) が置いた設計値） | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) REQ-D23-002（status: Tentative） |
| ながし | 軸馬を固定し、相手すべてと組み合わせる方式（軸が必須） | [CLAUDE.md](../../CLAUDE.md)「表記規約（最優先）」 |
| ボックス | 選んだ全馬を総当たりする方式（軸は不要） | 同上 |
| フォーメーション | 軸グループ×相手グループで組む方式。**このプロジェクトでは基本不使用** | 同上 |
| 均等割り配分 | 券種予算を脚数で割った 100 円単位の等額配分。賄えない端数の脚は ¥0。実装 `build_portfolio` が正 | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) REQ-D23-003 |
| ⚠ second source（買い方の二重実装） | 張るのは Rust `build_portfolio`（均等割り）の買い目。`scripts/predict-check/live_ev.py` は確率重み＋最低 ¥100 の別方式で、オフライン EV レポート専用 | [CLAUDE.md](../../CLAUDE.md)「予算・配分（既定）」 |

## 実行・運用

| 用語 | 要約 | 正本 |
|---|---|---|
| ⚠ degraded | **単複オッズの取得失敗だけ**が該当する状態。オッズ保存をまるごとスキップして **exit 3** を返す（出馬表・近走は保存済み） | [ADR 0049](../original-docs/0049-netkeiba-odds-transient-retry-and-degraded-exit.md) / [netkeiba-datasource.md](../specifications/netkeiba-datasource.md) |
| ⚠ 対象外スキップ | 障害レース等の取り込み対象外。DB 無変更で理由を stdout に出し **exit 0**。degraded（3）ともハード失敗（1）とも別 | [ADR 0075](../original-docs/0075-unsupported-race-skip-exit-zero.md) |
| best-effort | 失敗しても全体を巻き添えにしない経路（オッズ**未発売**・近走取り込み・組合せ券種オッズ）。**exit 0 のまま**なので件数はログで見る | [netkeiba-datasource.md](../specifications/netkeiba-datasource.md) |
| transient | リトライ対象の一過性障害（`Timeout` / `Io` / `ConnectionFailed` / `HostNotFound` / `Protocol` / 5xx）。最大 3 回・指数バックオフ | [ADR 0049](../original-docs/0049-netkeiba-odds-transient-retry-and-degraded-exit.md) |
| decision-support | ツールは判断材料を出すだけで、**張る / 見送り / 増額の最終判断は人間**という位置づけ。`predict-watch` の設計原則 | [ADR 0055](../original-docs/0055-ev-layer-separation-circular-break.md) / [ADR 0060](../original-docs/0060-betting-axis-lock-preclose-topup.md) / [CLAUDE.md](../../CLAUDE.md)「ライブ監視時のコミュニケーション規律」 |

## 文書運用の用語

蒸留モデルそのものの用語（3 層・`sources`・`distilled_from_sha`・stale・`doc_class`・REQ-ID）は
[README.md](README.md) と [doc-classes.md](doc-classes.md) が正本なので、ここでは重複させない。
