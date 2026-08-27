# ブラウザテストケース: 盤の被覆率差注記（#644）

対象: SPA `web/src/routes/RaceBoard.tsx`（ルート `/races/:raceId/board`）。
API は `GET /api/races/{race_id}/board` の `unpriced_legs` / `morning_unpriced_legs`。

検証環境は puppeteer-core + CDP レスポンス差し替えで board レスポンスの
`unpriced_legs` / `morning_unpriced_legs` を任意の値に加工して描画検証する。

### TC-11: 被覆率差ありのとき注記が表示される

| 項目 | 内容 |
|------|------|
| 前提 | board レスポンスで `morning_unpriced_legs=3`, `unpriced_legs=0`, `morning_roi` と `roi` が非 null |
| 画面 | `/races/{race_id}/board` |
| 操作 | 盤を表示し、朝ROI→現ROI の `<details>` を確認 |
| 期待結果 | summary 行に「※被覆率差」マーカーが表示される |
| 確認ポイント | `.coverage-diff` 要素が存在し、テキストに「被覆率差」を含む |

### TC-12: 被覆率差ありのとき展開で脚数が表示される

| 項目 | 内容 |
|------|------|
| 前提 | TC-11 と同じレスポンス |
| 画面 | `/races/{race_id}/board` |
| 操作 | 朝ROI→現ROI の `<details>` をクリックして展開 |
| 期待結果 | 展開内容に「朝 3 脚 → 現 0 脚」と「母集団が違う」旨の注記が表示される |
| 確認ポイント | `.coverage-diff-detail` 要素のテキストに脚数と注意文を含む。色は `var(--warn)` |

### TC-13: 被覆率差なしのとき注記が非表示

| 項目 | 内容 |
|------|------|
| 前提 | board レスポンスで `morning_unpriced_legs=0`, `unpriced_legs=0` |
| 画面 | `/races/{race_id}/board` |
| 操作 | 盤を表示 |
| 期待結果 | 「※被覆率差」マーカーと展開詳細が表示されない |
| 確認ポイント | `.coverage-diff` 要素と `.coverage-diff-detail` 要素が DOM に存在しない |

### TC-14: morning データなしのとき注記が非表示

| 項目 | 内容 |
|------|------|
| 前提 | board レスポンスで `morning_unpriced_legs=null`, `morning_roi=null` |
| 画面 | `/races/{race_id}/board` |
| 操作 | 盤を表示 |
| 期待結果 | 朝ROI→現ROI ブロック自体が非表示（morning データなしのため） |
| 確認ポイント | `.morning-roi-note` 要素が DOM に存在しない |
