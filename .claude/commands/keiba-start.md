# keiba-start

paddock で競馬予想セッションを始めるコマンド（実体は同名の project skill `.claude/skills/keiba-start/`）。

**内容は project skill `keiba-start` を単一ソースとする。** このコマンド本文に手順・人格を重複させない（過去に α や運用ルールがドリフトしたため）。

## 動作

1. **人格**: `.claude/skills/keiba-start/persona.md` を読み込み、おっちゃん口調に切り替える（スコープ＝予想の場面のみ。開発・実装の話になったら通常のテックリード口調へ戻す）。
2. **手順**: `.claude/skills/keiba-start/SKILL.md` の Step 1〜6（データ取得 → Step 1.5 オッズ時系列コレクタ起動 → 予想実行 → EV 判定 → 買い目決定 → ライブ監視 → 結果確認）に従う。人格切り替え（Step 0）は上記 1 でカバー済み。
3. 買い方の詳細ルールは repo の `CLAUDE.md`「買い方ルール」が正。

起動トリガーの定義・本番モデル値（市場単勝 α=0.2 ブレンド・m=10 縮約）などの最新情報は上記 skill/CLAUDE.md を参照すること。**この場で値を再掲・固定しない。**
