---
status: Confirmed
kind: knowledge
sources:
  - docs/qa/QA-live-freshness-382.md
  - docs/original-docs/382-live-server-now.md
  - docs/specifications/live-ev-buy-view.md
  - docs/adr/0064-live-ev-buy-view.md
distilled_from_sha: "8f8be21"
updated: "2026-07-22"
---

# ライブ盤 鮮度判定のサーバ時刻較正（#382）

ライブ盤（`web/src/routes/RaceList.tsx` / `web/src/lib/live.ts::freshness`）の鮮度バッジ（#372）は、
経過時間を**サーバ時刻基準で較正**する。クライアント時計のズレによる誤判定（遅れ→誤 stale・進み→誤 fresh）を避ける。

## 較正モデル（基準経過 + fetch 後補間）

- **基準経過** `base = server_now − last_updated`。両方サーバ由来の時刻なので**クライアント時計のズレを含まない**。
- **fetch 後経過** `localDelta = now − fetchedAt`。両方クライアント時計なので**絶対オフセットは相殺**し、
  純粋に「fetch 後にローカルで経過した時間」を表す（30 秒 tick で `now` が進むと表示だけ進み、閾値も正しく跨ぐ）。
- `diffMs = max(0, base + localDelta)`。`STALE_MINUTES=10` 超過かつ未発走レース残存で `stale`、未発走ゼロで `done`。
- **`fetchedAt` は React Query の `dataUpdatedAt`**（データ受領時のクライアント ms）。`now`（30 秒 tick）との差が
  fetch 後経過。

## API 契約（`server_now`）

- `GET /api/live/{date}` の `summary.server_now`（UTC rfc3339・秒精度）＝**レスポンス生成時のサーバ現在時刻**。
- handler（`handler/live.rs`）が `Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)` で生成し
  `LiveResponse::from_view(view, server_now)` に注入する。DB 由来の `view` は現在時刻を持たないため schema 層でなく
  handler が壁時計を注入する（関心分離・テスト可能）。

## フォールバック（非破壊）

- `server_now` が欠落/不正（旧サーバ・段階デプロイ）なら**クライアント時計にフォールバック**
  （`base = now − last_updated`、`localDelta = 0`＝#382 前の従来挙動）。graceful degradation。

## `freshness` シグネチャ

```ts
freshness(lastUpdated, serverNow, hasUpcoming, now, fetchedAt): Freshness
```

ユニットテスト（`web/src/lib/live.test.ts`）は較正の 3 系を検証: (1) クライアント時計 skew でも誤 stale にしない、
(2) fetch 後 tick で閾値を跨ぐ、(3) `server_now` null/不正でフォールバック。

## 背景

- #376 セルフレビューの follow-up。当時は「警告過多側=安全方向」で現状維持と判断していたのを #382 で較正。
- 鮮度バッジ・自動ポーリング自体は #372。ライブ盤の DTO 全体は ADR 0064 / `live-ev-buy-view.md`。
