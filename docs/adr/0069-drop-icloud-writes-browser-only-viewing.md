# 0069. iCloud 書き出しを全廃し、閲覧を REST API + SPA に一本化

## ステータス

承認済み（本 PR で実装）。対象 Issue: [#494](https://github.com/taito-station/paddock/issues/494)。

## コンテキスト

paddock は macOS の iCloud に 2 系統の書き込みをしていた。予想の閲覧が Obsidian の予想 MD からブラウザ（リッチ版フロント = `api-server` + React SPA）へ移行した結果、この iCloud 運用が不要になった。

- **(A) DB バックアップ iCloud ミラー**（`scripts/backup-db.sh` の `PADDOCK_BACKUP_MIRROR_DIR` 既定 = iCloud Drive `~/Library/Mobile Documents/com~apple~CloudDocs/paddock-backups`）。#494 で顕在化した穴: launchd 下では iCloud への列挙・削除が信頼できず（`cp` は効くが `ls`/`rm` が反映されない macOS file-provider の癖）、世代剪定が恒久 no-op になり dump が無制限に溜まる。ローカル権威 `~/paddock-backups`（`PADDOCK_BACKUP_DIR`）は KEEP=14 で常に剪定が効き、主脅威（Colima volume 喪失）は既にカバー済み。
- **(B) Obsidian/pad MD 書き出し**（`ingest-predictions --render` → iCloud Obsidian vault、唯一の消費者は legacy の `web-viewer`）。予想は DB が正で MD はその生成物（[prediction-json.md](../specifications/prediction-json.md)）。ブラウザ閲覧は DB 直読みの REST API + SPA（ADR 0022）で完結し pad MD に非依存。`--render` は launchd/スクリプトに組み込まれておらず手動実行のみ。

閲覧手段が REST API + SPA に統合された今、iCloud への書き込みは (A)(B) いずれも運用上の価値を失い、(A) は #494 の運用穴として残っていた。

## 決定

**iCloud への書き込みを全廃し、閲覧は REST API + SPA、バックアップはローカル権威に一本化する。**

1. **DB バックアップの iCloud ミラーを既定 off にする**（#494 解消）。`PADDOCK_BACKUP_MIRROR_DIR` の既定を iCloud パス → **空文字（無効）**に変更。off-machine ミラーは env を明示指定したときだけ有効で、指定先は**実ファイルシステム（外付け/NAS 等）**とする（iCloud は使わない）。ミラー＋剪定コードは汎用処理として残す（既定 off なので通常経路は iCloud に一切触れない）。
2. **Obsidian/pad MD 書き出しパイプラインを廃止する**。`ingest-predictions` から `--render` / `render_all` / `render.rs` / `DEFAULT_PAD_DIR` / 関連 CLI フラグを削除。render 専用の repository メソッド `list_pad_predictions` を trait/gateway から削除。pad MD の唯一の消費者だった `web-viewer` crate（`paddock-web`）を workspace ごと削除。JSON→DB の取り込み（`save_pad_prediction`）は不変。

## 理由

- **#494 を構造的に解消する**。iCloud を既定ミラー先から外せば「launchd 下で剪定が no-op で溜まる」経路自体が消える。reconcile のリマインダー（issue の代替案）を運用に足すより、原因（iCloud のミラー既定化）を除去する方が「一時的な修正をしない」「課題を後回しにしない」に適う。
- **閲覧は既に REST API + SPA に一本化されている**。web-viewer は DB 非依存で pad MD を読むだけの legacy ビューアであり、pad MD を生成しなくなれば無入力の死コードになる。MD 生成（`--render`）とその唯一の消費者（web-viewer）を同時に畳むのが最小構成。
- **DB が正の原則を崩さない**。予想の永続化は DB（`predict_sessions`/`predict_bets`）で完結しており、MD は派生生成物だった。MD を廃しても予想データと閲覧は損なわれない。

## 影響

- **削除**: `ingest-predictions` の `--render`/`render.rs`/`DEFAULT_PAD_DIR`／repository `list_pad_predictions`（trait + gateway）／`web-viewer` crate（workspace member と workspace 依存 `pulldown-cmark`）。ドキュメント（README の web-viewer 節・`--render` 記述、`scripts/predict-check/gen_predictions.py` のコメント）。
- **変更（既定値）**: `scripts/backup-db.sh` の `PADDOCK_BACKUP_MIRROR_DIR` 既定を空へ。`deployments/db/BACKUP.md` を「ローカル権威一本、ミラーは非iCloud opt-in」に更新。launchd plist / `install.sh` はミラー系 env を元々注入していないため変更不要。
- **不変**: DB バックアップのローカル権威退避・世代剪定（`~/paddock-backups`・KEEP=14）／`ingest-predictions` の JSON→DB 取り込み／REST API + SPA（`api-server` / `web/`）による閲覧／compose の `web` サービス（React SPA。`paddock-web` という image 名が web-viewer バイナリ名と偶然衝突していたが別物）。
- **トレードオフ**: DB dump の off-machine 冗長（ディスク障害対策）を**既定で失う**。ローカル権威が主脅威（Colima volume 喪失）を外す一方、ディスク障害時は別途 off-machine ミラー（非iCloud パスを env 指定）が必要。実ファイルシステムへの自動ミラー化は将来 issue に委ねる。
- **既存 iCloud 資産の掃除**: 既に iCloud に溜まった DB dump（`.../CloudDocs/paddock-backups`）と予想 MD vault（`.../iCloud~md~obsidian/.../pad`）は手動で削除する（不可逆のためスクリプト化せず運用者が実行）。
- 関連: #265（DB バックアップ）／ADR 0022（REST API read）／#143（web-viewer）／#34（Web SPA）。
