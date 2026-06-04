# 0004. predict バイナリの対話セッション設計

## ステータス

提案中

## コンテキスト

Issue #12 の実装（PR #19、main にマージ済み）で Domain 層の EV 計算・Kelly 配分ロジックが完成した。
次のステップとして、1 日の開催を順番に処理し、ユーザーが買い目を確認・購入記録する対話型 CLI が必要。

## 決定

- 新規バイナリ `paddock-predict`（`src/apps/predict`）を追加する
- 起動引数: `--date YYYY-MM-DD --budget 金額`
- レースごとに確率推定 → オッズ取得 → 買い目推奨 → ユーザー選択（y/e/s）→ 払い戻し入力 → 残高更新 のループを繰り返す
- セッション状態（残高・累計）は App 層のみで `u64` として管理し、残高ガードで 0 以上を保証する
- 確率推定・レース一覧・オッズ取得は Use-Case の Interactor 経由、`select_bets`（純粋関数）は App 層が `paddock-domain` から直接呼ぶ
- `Repository` トレイトに `find_races_by_date` と `find_race_odds` を追加する
- オッズ未取得（`find_race_odds` が `None`）のレースは買い目推奨を生成せず、スキップのみを受け付ける

## 理由

- 既存の `paddock-analyze predict <race_id>` は単レースの確率表示に特化しており、1 日セッション管理の責務を持たせるのは不適切
- App 層が対話 IO とセッション状態を持つことで Domain/Use-Case の純粋性を維持できる
- `select_bets` は IO を伴わない純粋関数のため、薄い委譲を避けて App 層から直接呼ぶ
- `find_races_by_date` は `races` テーブルの日付による単純なクエリで追加コストが小さい

## 影響

- `Repository` トレイトに 2 メソッド追加 → `rdb-gateway` と（将来の）モックに実装が必要
- **`race_odds` テーブルは現状未存在**。オッズの永続化（テーブル追加 + スクレイパー保存）は別 Issue のスコープとし、本 Issue では `find_race_odds` が `None` を返すケースを正しくハンドリングする（買い目推奨なし → スキップ）
- ワークスペース `Cargo.toml` に `src/apps/predict` メンバーを追加
- 既存の `paddock-analyze` への変更はない
