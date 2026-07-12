# ブラウザテストケース: レース一覧（日次ダッシュボード）

対象: SPA `web/src/routes/RaceList.tsx`（ルート `/races?date=YYYY-MM-DD`）。設計書 [web-spa.md](../../docs/specifications/web-spa.md) / [rest-api-read.md](../../docs/specifications/rest-api-read.md)。

検証環境は Playwright MCP 不在のため **headless Chrome + puppeteer-core** で代替する（`reference_browser_test_fallback`）。

---

### TC-01: watch 未判定レースの発走時刻表示と状態分類（#391）
| 項目 | 内容 |
|------|------|
| 前提 | race_cards に post_time があるが live_ev_snapshots に判定記録が無いレースが存在する（例: watch 起動前に発走済みの 1R、監視窓未到達の最終レース）。現在時刻がその post_time より後のケースと前のケースを両方用意 |
| 画面 | `/races?date=YYYY-MM-DD` |
| 操作 | 状態フィルタ「全部」「未発走」「終了」を切り替える |
| 期待結果 | watch 未判定でも発走列に race_cards 由来の HH:MM が表示される。post_time が過去のレースは ⚫終 マークが付き「終了」フィルタに含まれ、「未発走」フィルタに混ざらない。post_time が未来のレースは「未発走」側。ROI・軸など EV 列は従来どおり「—」のまま |
| 確認ポイント | post_time が null（出馬表未取得）のレースは従来どおり未発走側に残る（終了と断定しない）。EV 数値が捏造表示されないこと |
