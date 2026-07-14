# QA: #382 ライブ鮮度判定のサーバ時刻較正

> 質問票+回答（[docs/qa/README.md](README.md)）。一次資料は [docs/original-docs/382-live-server-now.md](../original-docs/382-live-server-now.md)。
> 回答済みの本票は [docs/knowledge/live-freshness-calibration.md](../knowledge/live-freshness-calibration.md) に蒸留した。

## Q1: 経過時間の較正モデルは

- 観測/根拠: 旧 `freshness` は `now(クライアント時計) - last_updated`。クライアント時計が遅れると誤 stale、
  進むと fresh 側（0 クランプ）に倒れる。要件は「`server_now - last_updated` で経過、クライアント時計は
  相対表示の補間のみ」。
- 回答: **基準経過と fetch 後補間を分離する**。
  - **基準経過** `base = server_now − last_updated`（ともにサーバ由来なのでクライアント時計のズレを**含まない**）。
  - **fetch 後経過** `localDelta = now − fetchedAt`（ともにクライアント時計の**相対差**なので絶対オフセットは相殺）。
  - `diffMs = max(0, base + localDelta)`。30 秒 tick で `now` が進むと表示だけ進み、閾値も正しく跨ぐ。
- 反映先: `web/src/lib/live.ts::freshness`、`src/interface/rest-controller/src/schema/live.rs`（`server_now` 追加）。

## Q2: `server_now` はどこで生成し、どの精度で載せるか

- 観測/根拠: `server_now` はレスポンス生成時の壁時計で、DB 由来の `view` には無い。`captured_at` は UTC rfc3339。
- 回答: **handler が `Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)` で生成し `from_view(view, server_now)`
  で注入**。schema 層に現在時刻を持ち込まない（テスト可能・関心分離）。秒精度・UTC・`captured_at` と同形式。

## Q3: fetch 後経過の基準（fetchedAt）に何を使うか

- 観測/根拠: web は `useQuery`。`live.dataUpdatedAt`（データ受領時のクライアント ms）が取れる。
- 回答: **React Query の `dataUpdatedAt` を `fetchedAt` に使う**。`now`（30 秒 tick）との差 `now − dataUpdatedAt` が
  「fetch 後にローカルで経過した時間」。両者ともクライアント時計なので絶対オフセットは相殺し較正が保たれる。

## Q4: `server_now` が読めないとき（旧サーバ等）

- 観測/根拠: 段階デプロイや古い API では `server_now` が欠落/不正でありうる。
- 回答: **クライアント時計にフォールバック**（`base = now − last_updated`、`localDelta = 0`＝従来挙動）。
  非破壊で graceful degradation。ユニットテストで null/不正双方を検証。
