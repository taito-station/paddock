# CLI テストケース: predict コマンド

`paddock-analyze predict <race_id>` の動作確認手順。

## TC-01: 正常ケース（全馬にスタッツあり）

| 項目 | 内容 |
|------|------|
| 前提 | DBに race_card と horse/course/jockey スタッツが十分に蓄積されている |
| コマンド | `paddock-analyze predict <有効な race_id>` |
| 期待結果 | 馬番・馬名・勝率・連対率・複勝率の表が表示される |
| 確認ポイント | 各確率が 0.0〜1.0 の範囲内 / 勝率の合計が ≒ 1.0 / 連対率の合計が ≒ 1.0 |

## TC-02: 存在しない race_id を指定

| 項目 | 内容 |
|------|------|
| 前提 | 指定する race_id が DB に存在しない |
| コマンド | `paddock-analyze predict 9999999999` |
| 期待結果 | `not found` 等のエラーメッセージが stderr に出力され、exit code 1 |
| 確認ポイント | パニックせずにエラーハンドリングされること |

## TC-03: スタッツ件数ゼロの馬が含まれる（均等フォールバック）

| 項目 | 内容 |
|------|------|
| 前提 | 出走馬のうち 1 頭以上の過去成績が 0 件 |
| コマンド | `paddock-analyze predict <対象 race_id>` |
| 期待結果 | エラーにならず確率が表示される（フォールバック: 均等確率 1/頭数） |
| 確認ポイント | スタッツなし馬も行として出力されること / 合計確率が ≒ 1.0 |

## TC-04: 騎手なし（jockey = None）の馬が含まれる

| 項目 | 内容 |
|------|------|
| 前提 | HorseEntry の jockey フィールドが None の馬がいる |
| コマンド | `paddock-analyze predict <対象 race_id>` |
| 期待結果 | 騎手スタッツなしでスコア計算され、結果が表示される |
| 確認ポイント | 騎手なしでもクラッシュしないこと |

## TC-05: race_card が未保存の race_id を指定

| 項目 | 内容 |
|------|------|
| 前提 | races テーブルには存在するが race_card_entries が未保存の race_id |
| コマンド | `paddock-analyze predict <race_id>` |
| 期待結果 | `race card not found` 等のエラーが stderr に表示される |
| 確認ポイント | 空の結果セットが返らず、明示的なエラーになること |
