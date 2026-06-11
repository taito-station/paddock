# ADR 0013: 予想セッションの馬場入力を永続化し再現可能にする (Issue #80)

## ステータス
承認済み

## コンテキスト
#73（ADR 0011）で予想セッションは各レース冒頭に馬場状態（良/稍重/重/不良）を対話入力する
ようになったが、入力値はその場で `predict_race` に渡されるのみで**どこにも永続化されない**。
未確定レースの `races.track_condition` は構造的に NULL（値が入るのは成績取り込み後）のままなので、
事後に「どの馬場前提でこの確率・買い目を出したか」を**再現・監査できない**。PR #79 のセルフレビュー
（2 巡目）で検出し、別 Issue 化していた。

設計上の論点:
- 記録の単位（`predict_bets` への列追加 / `predict_sessions` への列追加 / レース単位の記録テーブル）。
- 「不明として入力した」状態と「未入力」をどう区別するか。
- `--resume` 再実行時にどの値をデフォルト提示するか（記録値・直前入力・確定値の優先順）。

## 決定

1. **レース単位の記録テーブル `predict_race_conditions` を新設**する。
   ```sql
   CREATE TABLE predict_race_conditions (
       session_date    TEXT NOT NULL REFERENCES predict_sessions(date) ON DELETE CASCADE,
       race_id         TEXT NOT NULL,
       track_condition TEXT,                       -- 良/稍重/重/不良。NULL=不明として記録
       created_at      TEXT NOT NULL,
       updated_at      TEXT NOT NULL,
       PRIMARY KEY (session_date, race_id)
   );
   ```
   - **行の存在 = そのレースで入力済み**、`track_condition IS NULL` = 「不明として入力済み」。
     未入力（行なし）と明確に区別する。
   - `predict_bets` への列追加にしない理由: 買い目は組み合わせ単位で複数行に重複し、かつ
     **買い目が無い／スキップしたレースでは馬場入力が一切残らない**。`predict_sessions`（1 日 1 行）
     にも入らない。レース単位テーブルなら買い目の有無に依存せず 1 レース 1 行で監査が明瞭。

2. **入力直後に必ず保存する**。`read_track_condition` の直後（確率推定・オッズ取得より前）に
   upsert するため、出馬表未登録（NotFound）・オッズ未取得・スキップでも入力値が残る。
   セッションヘッダ（`predict_sessions`）は `save_race_outcome` と独立に更新されるので、
   馬場入力も独立した `save_predict_race_condition` で書き込む。

3. **`--resume` 時のデフォルト提示は優先順「記録済みの値 → 同一セッション内の直前レース入力 →
   `races` の確定値」**で決める（純関数 `resolve_track_condition_default`）。
   - 記録済み（resume）の値は最優先。`None`（不明として記録）も維持し、フォールバックしない。
   - 未記録のレースのみ、同一セッション内の直前レース入力をデフォルト提示する。芝/ダ・日中の
     馬場変化があるため**自動適用はせずデフォルト提示に留める**（空入力で採用、`-` で不明を明示）。
   - 直前入力も無ければ `races.track_condition`（通常 None）にフォールバック。

4. **記録時刻は use-case 層で注入**し、gateway を時計から独立に保つ（`FetchRecord` と同じ流儀）。
   upsert は `ON CONFLICT(session_date, race_id) DO UPDATE` で `created_at` を初回値のまま保持する。

## 理由
- 「どの馬場前提で予想したか」を後から再現・監査できるようにすることが本 Issue の目的で、
  買い目の有無に依存しないレース単位テーブルが最も素直に要件を満たす。
- `None` を「不明として入力済み」とし行の存在で「入力済み」を表すことで、`read_track_condition` の
  既定挙動（空入力＝デフォルト維持、デフォルト None なら不明）とそのまま往復する。
- upsert（`ON CONFLICT DO UPDATE`）方式は `predict_sessions`/`race_odds` の既存流儀と一貫する。

## 影響
- 新テーブル `predict_race_conditions` とマイグレーション 1 本を追加。
- `Repository` トレイトに `find_predict_race_conditions` / `save_predict_race_condition` を追加
  （全モック実装の追従が必要）。
- `apps/predict` のセッションは、レース冒頭で馬場入力を保存し、`--resume` で記録値・直前入力を
  デフォルト提示するようになる。`races` / `predict_bets` / 既存の確率推定ロジックには影響しない。
- `predict_race` のシグネチャは変更しない（本 ADR は永続化のみで、確率推定への配線は ADR 0011 のまま）。

## 関連
- ADR 0011（馬場状態を確率推定に接続 / 対話入力の導入）— 本 ADR が永続化を補完
- #73 / PR #79（レビュー対応履歴の 2 巡目・5 巡目）
- 設計書 `docs/specifications/predict-session.md`
