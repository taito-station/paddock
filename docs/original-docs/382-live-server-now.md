# 原資料: #382 ライブEVレスポンスにサーバ時刻を含め鮮度判定を較正する

> 一次資料（RO 生素材）。蒸留は QA（[QA-live-freshness-382](../qa/QA-live-freshness-382.md)）→ knowledge で行う。

## Issue #382 概要

ライブボードの鮮度判定（`freshness`, `web/src/lib/live.ts`）は `summary.last_updated`（サーバ保存時刻）と
**クライアント時計**の比較で行っており、クライアント時計のズレで誤判定しうる（遅れ側は誤 stale 警告、
進み側は 0 クランプで fresh に倒れる設計済み）。サーバ時刻を基準に較正する。

### 要件
- `GET /api/live/{date}` のレスポンス（summary）にサーバ現在時刻（`server_now` 等、rfc3339）を追加
- web 側は `server_now - last_updated` で経過を計算し、クライアント時計は相対表示の補間（30 秒 tick）にのみ使う
- `freshness` のシグネチャ変更とユニットテスト更新

### 補足
- PR #376 セルフレビューの follow-up（現状は「警告過多側=安全方向」として現状維持と判断したもの）
- 関連: #372（鮮度バッジ・自動ポーリング）, #376

## コード調査所見（2026-07-14, worktree paddock-382 = main 9f35d93 起点）

### サーバ現状
- `src/interface/rest-controller/src/schema/live.rs::LiveSummary`: `bet_race_count` / `watched_race_count` /
  `last_updated`（`captured_at` 最大値）。`server_now` は無い。
- `LiveResponse::from_view(view)` が use-case の `LiveView`（DB 由来）を写像。現在時刻は view に無い。
- `src/interface/rest-controller/src/handler/live.rs::get_live`: `find_live_by_date` → `from_view` → 200。

### web 現状
- `web/src/lib/live.ts::freshness(lastUpdated, hasUpcoming, now)`: `now.getTime() - lastUpdated` で経過算出。
  `now` は RaceList の 30 秒 tick（`useState(new Date())` + `setInterval`）由来。
- `web/src/routes/RaceList.tsx`: `freshness(summary.last_updated, hasUpcoming, now)` を呼ぶ。`live` は
  `useQuery`（`live.dataUpdatedAt` = データ受領時のクライアント ms が取れる）。
- 鮮度は `STALE_MINUTES=10` 超過かつ未発走レース残存で `stale`、未発走ゼロで `done`。

## 未確定（QA で解消）
- 較正の計算モデル（基準経過と fetch 後補間の分離）。
- `server_now` 欠落時のフォールバック。
- fetch 後経過の基準に何を使うか（React Query `dataUpdatedAt` 等）。
