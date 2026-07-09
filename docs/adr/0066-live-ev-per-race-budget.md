# 0066. ライブ EV 伝票の per-race 予算（増額）は predict-watch の CLI override で入力し slip に記録する

## ステータス

承認済み（実装 PR に本 ADR を同梱）。対象 Issue: [#342](https://github.com/taito-station/paddock/issues/342)。関連: ADR 0055（EV 層分離）・0060（軸ロック＋ズレ増額）・0064（ライブ EV 買い目ビュー・writer を Rust `predict-watch` に一本化）。

## コンテキスト

ライブ EV 買い目伝票（`predict-watch` → `live_ev_snapshots` → SPA `/live/:date`）は全レース予算が **¥5,000 固定**で、`slip.race_budget` は「将来の per-race 予算差分の予約枠」として未活用だった。

CLAUDE.md 買い方ルールには「**+EV レースは増額してよい（唯一エッジがある局面）**」があり、ADR 0060 は「**発走直前オッズの用途はズレ増額のみ**（軸・点数・相手は不変、金額だけ上げる）」と定める。しかし現状は増額の"きっかけ"（🔶ズレ）を表示するだけで、**増額後の金額を伝票に反映する経路が無い**。

per-race 予算を「どこに・どう持たせるか」は既存 ADR の思想と衝突しうる:

- **ADR 0060（軸ロック＝decision-support）**: 増額は人間の執行判断であり、モデル確率・基準配分に戻さない。→ per-race 予算をモデル側の出力として持たせると「モデルが増額を計算する」ことになり軸ロックの思想と競合する。
- **ADR 0064（SPA は描画のみ・計算は正本 `predict-watch`）**: SPA 側で金額を再配分するのは禁止（二重実装・乖離リスク）。→ 増額後の金額は必ず正本側で計算する必要がある。

## 決定

**Approach C: per-race 予算は `predict-watch` の CLI override（`--race-budget-override <race_id>=<円>`）で人間が明示入力し、指定レースだけその予算で `build_portfolio` を回して `slip.race_budget` に記録する。**

- 入力: `predict-watch` に `--race-budget-override` を追加（`<race_id>=<円>` 形式・複数レースはフラグ繰り返し）。起動時に形式検証（`RaceId` 形式・予算 **≥100 円**・重複禁止）し、適用一覧を表示。当日レースに一致しない race_id は初回スイープの出馬表を基準に 1 度だけ警告する。予算 100 円未満は `build_portfolio` の券種予算 floor で空伝票になるため弾く。
- 計算: 指定レースは override 予算、未指定レースは既定 `--race-budget` を使って `build_portfolio` に渡す。**予算は配分額（各点の金額）にのみ効き、軸・点数・相手（3 券種とも top5）は不変**（`build_portfolio` の仕様どおり）。
- 記録: `SnapshotContext.race_budget` に per-race 値を詰め、既存経路で `slip.race_budget`（`live_ev_snapshots.slip` JSONB）に保存。**DB スキーマ変更なし**。
- 描画: SPA は `slip.race_budget` と各 leg の金額をそのまま描画（既存実装・再配分しない）。

## 理由

- **軸ロック（ADR 0060）と整合**。増額は「人間が CLI で明示指定した執行入力」であって、モデルが計算した基準配分ではない。モデルの確率推定・順位付け（軸選定）は一切不変で、金額だけが人間の判断で上乗せされる。snapshot に残る `race_budget` は decision-support の観測記録（「このサイクルで人間はこの予算で執行意図した」）であり、モデル原本の書き換えではない。
- **EV 層分離（ADR 0055）と整合**。EV は純モデル×市場 odds で計算し、増額の可否は人間が EV/ROI を見て判断する。予算入力は計算の外側（CLI）にあり EV 計算を汚さない。
- **正本一本化（ADR 0064）と整合**。増額後の金額計算は正本 `predict-watch`（`build_portfolio`）だけで行い、SPA は描画に徹する。2 エンジン（Python 正本の復活）を招かない。
- **最小変更**。`build_portfolio(probs, budget)` は budget を配分額にのみ使う既存設計のため、per-race 値を差し込むだけで済む。スキーマ・API・SPA 契約は不変。

## 棄却した代替案

- **A. 原本にモデル配分として保存**（DB に per-race 予算スカラー列を追加し、モデル側が per-race 予算を持つ）: ADR 0060 に違反。モデルが per-race 予算を持つと EV/配分の再計算圧力が生まれ、「増額は人間の執行判断」の分離が崩れる。
- **B. 原本は基準固定・増額は表示 overlay のみ**（snapshot は常に ¥5,000、SPA 側で増額表示）: 増額後の金額を算出する主体が必要だが、ADR 0064 が SPA 再配分を禁止するため結局正本側の per-race 計算が要る＝ C に収束する。overlay 単独では金額を出せない。
- **D. `live_ev.py`（Python）に per-race map 入力を戻す**: ADR 0064 の「writer を Rust `predict-watch` に一本化」を覆し 2 エンジン問題を再燃させる。アーキ劣化。

## 影響・結果

- `predict-watch` の CLI に `--race-budget-override` が増える。既定挙動（override なし）は従来と完全に不変（全レース `--race-budget`）。
- `slip.race_budget` が per-race 値を取りうるようになる（従来は `default_budget` と同値固定）。SPA は既存描画で per-race 金額を表示できる。
- 運用フロー: 朝に軸決定 → ライブ監視で 🔶 増額候補を検出 → 人間が判断 → `--race-budget-override <pid>=<円>` を付けて再実行 → snapshot に増額伝票が記録され SPA に反映。
- スコープ外: 混戦判定・配分ロジック（最大剰余法・3 券種・top5）は不変（ADR 0046/0065）。予算の 100 円単位への切り捨ては `build_portfolio` の既存挙動に委ねる。
