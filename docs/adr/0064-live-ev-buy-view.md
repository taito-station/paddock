# 0064. ライブ EV 買い目ビューを SPA に追加（Python 伝票を正本に永続化 → read API → SPA 描画）

## ステータス

提案中（設計書 PR レビュー中）。対象 Issue: [#260](https://github.com/taito-station/paddock/issues/260)。**本設計書 PR のマージ承認をもって「承認済み」に更新**する。本 ADR に伴う実装は承認後の別 PR（API → SPA の順）。

## コンテキスト

開催当日のライブ監視で「**結局いま何を買えばいいのか**」を毎回見失う。現状は手作業の買い目シート（`買い目_YYYYMMDD.md`）を 20 分サイクルごとに人手更新しており、最新サイクルの「張る/見送り」と「そのまま買える買い目」を一望できる場所が無い。

- 「張る/見送り＋そのまま買える伝票」を出すのは `scripts/predict-check/live_ev.py --slip`（`refresh_ev.sh` が駆動）だが、**出力は CLI/標準出力のみ**で UI に出ていない。ライブ中はターミナル出力を見て md を手写しする運用になり、前サイクルの古い買い目と混ざる。
- SPA（`web/src/routes/`）と REST API は**事後のセッション＋outcome 記録**向けで、ライブ監視フローを想定していない。
- 既存 API `/api/races/{race_id}/recommendations` は use-case `recommend_bets()` → `build_portfolio()`（Harville・一律 top5・**混戦判定なし**）で、CLAUDE.md「買い方ルール」準拠の `live_ev.py` 伝票（Plackett-Luce・混戦ボックス・相手 top3/top5 分別・最大剰余法配分）**とは別物**。今の API では「そのまま買える伝票」を出せない。

CLAUDE.md「買い方ルール」（混戦判定・相手幅・配分）の一次定義は CLAUDE.md・実装は `live_ev.py`。関連 ADR 0028/0030/0046 は**代替案を棄却して baseline を固定した記録**（`*-rejected.md`）であり定義そのものではない。EV 層分離と軸ロック（decision-support）は ADR 0055/0060。ライブ監視の伝票を UI 化するにあたり、この確定ロジックをどこに正本として置くかが論点。

## 決定

**Approach C: ライブ EV/伝票ロジックの正本は Python `live_ev.py` に一本化し、UI へは永続化 snapshot 経由で公開する。**

1. `live_ev.py` に `--emit-json` を追加（**出力追加のみ・計算は不変**）。各監視サイクルの ROI・張る/見送り判定・買い目伝票（式別/方式/軸/相手/点数/金額）を機械可読 JSON で出力する。
2. `refresh_ev.sh`（DB アクセスを持つオーケストレータ）が、その JSON を Postgres の新テーブル `live_ev_snapshots` へ upsert する（サイクルごとの時系列アーカイブ、`race_odds_snapshots` #232 と同思想）。
3. read-only API `GET /api/live/{date}` を追加。**race ごと最新サイクルのみ**を返し、直前 snapshot 比較でフリップ（◎変化・+EV↔−EV 反転）を算出、トップに一望サマリ（張る本数・監視数・最終更新時刻）を付ける。
4. SPA に `LiveBets`「今これを買え」ビューを追加。**描画のみ**（張る＝そのまま買える伝票 / 見送り＝理由付き / フリップ強調 / 最新サイクルのみ正）。

## 理由

- **買い方ルールを二重実装しない**。混戦判定・Plackett-Luce・相手 top3/top5 分別・最大剰余法配分は ADR 0028/0030/0046/0055/0060 で確定済み。これを Rust domain（Approach A）や TS に再実装すると、確定ロジックの second source が生まれ乖離する。正本を `live_ev.py` 単一に保つのが「シンプル第一」「一時的な修正をしない」に適う。
- **「最新サイクルのみが正」を構造で表現できる**。サイクルごと snapshot を時系列で持てば、最新 = `max(captured_at)`、フリップ = 直前との差分で自然に導ける。前サイクル/朝の +EV を UI に混ぜない CLAUDE.md 規律をデータ構造が担保する。
- **SPA の鮮度方針と整合**。web-spa.md は「永続化済みデータを表示・自動ポーリングしない」。本ビューも snapshot 済みを描画するだけで philosophy を崩さない。
- **既存周期に相乗り**。`refresh_ev.sh` は既に 20 分周期で `live_ev.py` を駆動しており、永続化 1 ステップの追加で済む。実装最小。

### 代替案と棄却理由

- **Approach A（Rust domain へ移植）**: `/races/{id}/live-slip` で API がオンデマンド算出。クリーンアーキ的に自己完結だが、確定済み買い方ルールの二重実装＝乖離リスクが最大。棄却。
- **Approach B（API が `live_ev.py` を都度 subprocess 実行）**: Rust サーバが Python + TSV パイプラインに実行時依存し脆い。運用障害点が増える。棄却。

## 影響

- **新規**: Postgres テーブル `live_ev_snapshots`（マイグレーション）／`live_ev.py --emit-json`（＋テスト）／`refresh_ev.sh` に永続化ステップ／read API `GET /api/live/{date}`（rest-controller・use-case・rdb-gateway・api-server の 4 層＋utoipa snapshot 検証）／SPA `LiveBets` ビュー 1 画面。
- **不変**: `live_ev.py` の計算ロジック（買い方ルール・ROI・混戦判定）／既存 `/api/races/{race_id}/recommendations`（`recommend_bets()`→`build_portfolio()`）／確率モデル・EV 層（ADR 0055）／予算・配分（ADR 0046）。
- ライブ監視の運用が「ターミナル＋手写し md」から「UI 一望」へ移行し、最新サイクル散逸・前サイクル混入のヒューマンエラーが消える。あくまで decision-support（ADR 0055/0060）で、張る/見送り/増額の最終判断・軸ロックは人間側に残る。
- 関連: 0028・0030（混戦判定・相手幅）／0046（配分・floor）／0055（EV 層分離・decision-support）／0060（軸ロック＝ズレ増額のみ）。設計詳細は [docs/specifications/live-ev-buy-view.md](../specifications/live-ev-buy-view.md)。
