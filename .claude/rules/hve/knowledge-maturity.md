---
description: Knowledge 成熟度モデル — frontmatter 標準・status 管理・SoT 優先順位・Stale 検出の仕組み。
---

# Knowledge 成熟度モデル

knowledge/ 配下の文書の信頼度と鮮度を管理する仕組み。

## frontmatter 標準

knowledge/ 配下の各ファイルは以下の frontmatter を持つ（決定ログの `knowledge/adr/` と `README.md` は対象外。ADR の様式は documentation-standards に従い、同スキルが無ければ artifact-management の「書式」の読み替えに従う）:

```yaml
---
title: 文書タイトル
status: Confirmed | Tentative | Conflict
kind: knowledge | specification
sources:
  - docs-original/NNN-xxx.md
  - qa/QA-yyy.md
distilled_from_sha: "a1b2c3d4..."  # 蒸留時点のコミット。フル SHA（40 文字）。文書ごとに 1 つ
updated: "YYYY-MM-DD"
doc_class: プロジェクト定義の分類コード
tags: [分類コード]
---
```

### 各フィールドの定義

| フィールド | 必須 | 説明 |
|---|---|---|
| `title` | Yes | 文書タイトル |
| `status` | Yes | 信頼度（下記参照） |
| `kind` | Yes | `knowledge`（ドメイン知識）または `specification`（仕様） |
| `sources` | Yes | 蒸留元ファイルのパス一覧 |
| `distilled_from_sha` | Yes | 蒸留時点のコミットの sha（文書ごとに 1 つ。下記「Stale 検出の仕組み」参照） |
| `updated` | Yes | 本文または status を実質的に更新した日（ISO 8601、ダブルクォート必須）。下記「`updated` の規則」参照 |
| `doc_class` | No | 文書の役割分類（プロジェクトが定義する） |
| `tags` | No | 検索用タグ |

### `updated` の規則

`updated` は、本文または status を実質的に更新した日を示す。本文または status が実質的に変わったときだけ当日日付に進め、`distilled_from_sha` だけを追従させるときは触らない。下流の本文に効かない上流の変更まで日付を進めると、「いつ内容が変わったか」という信号が濁る。

| 変更 | `distilled_from_sha` | `updated` |
|---|---|---|
| source の誤字修正など、下流の本文に効かない上流の変更 | 進める | 据え置く |
| 本文の事実・判断が変わる差分マージ | 進める | 進める |
| Tentative → Confirmed への昇格だけ | 据え置く | 進める |
| Conflict の宣言（下記「status の 3 段階」参照） | 蒸留前の値のまま | 進める |
| knowledge 本文の、意味の変わらない表記揺れ・誤字の修正 | 据え置く | 据え置く |

値は必ずダブルクォートで囲む（`updated: "2026-10-04"`）。クォートしないと YAML が date 型に解釈し、文字列を前提にした JSON 化や比較で型が揺れる。HVE の mdq も `default=str` の導入前（HypervelocityEngineering 8ab5a42 より前）は、frontmatter の JSON 化が `Object of type date is not JSON serializable` で失敗し、索引化できなかった。

## status の 3 段階

| status | 定義 | 運用 |
|---|---|---|
| `Confirmed` | 確定知。運用の前提にしてよい | QA で Confirmed された回答、またはレビュー済みの蒸留結果 |
| `Tentative` | 暫定。前提に使えるが変更の可能性あり | 初回蒸留の結果、未レビューの推論補完 |
| `Conflict` | 矛盾あり。放置せず解消が必要 | source 間の矛盾、または蒸留時に検出した不整合 |

- `Conflict` は発見次第ユーザーに報告し、解消するまで該当部分を前提にしない
- `Conflict` の文書は、本文と `distilled_from_sha` を蒸留前の状態に保つ。同じ回の蒸留で差分マージしていたら、本文と sha をどちらも蒸留前に戻し、status と `updated`（宣言した日）だけを変える。こうすると解消時に、蒸留前の sha から最新の source までの差分をそのまま取り込める
- `Conflict` の文書は解消するまで蒸留しない。ユーザーが矛盾を判断して status を戻したら、次回の蒸留で通常どおり差分マージし、sha を進める
- すでに `Conflict` の文書は、整合性レビューで矛盾が見つかっても宣言し直さない（status・本文・`distilled_from_sha`・`updated` を変えない）。解消待ちとして報告するだけにする
- `Tentative` → `Confirmed` への昇格は、QA 回答の確認またはユーザーのレビュー承認による

## SoT（Single Source of Truth）優先順位

情報が矛盾する場合、上位が勝つ:

```
docs-original（一次資料）> qa（Confirmed 回答）> knowledge（蒸留済み確定知）
```

- `docs-original/`: ユーザー提供の原本。最も信頼度が高い
- `qa/`: 質問票に対するユーザーの回答。Confirmed ステータスのもの
- `knowledge/`: 上記を蒸留・統合した文書。派生物であり、source に従う

## Stale 検出の仕組み

knowledge 文書が stale（source より古い）かどうかを、frontmatter の `distilled_from_sha` を基準に判定する。判定と追従は、setup.sh が `.claude/scripts/hve/` に配るスクリプトで行う（hve-playbook 自身では `hve-scripts/`）。

### 判定ロジック（祖先判定）

1. 文書の `distilled_from_sha` を `git rev-parse --verify` でコミットに解決する（短縮形も可）
2. 各 source について「最後に内容が変わったコミット」を求める。次のコミットは内容の変更と見なさず、さらに遡る
   - 内容を変えないリネーム（R100）。リネーム元のパスで遡り続ける
   - frontmatter のメタデータ（`doc_class`・`tags`・`sources`・`distilled_from_sha`・`updated`）だけの変更。`status` と `kind` の変更は内容の変更とする
   - `.github/workflows/` の `uses: owner/repo@<40 桁>` のピン更新だけの変更
3. そのコミットが `distilled_from_sha` の祖先（または同一）でなければ stale

source の sha と一致するかで比べないのは、リネームやメタデータだけの変更で偽の stale を出さないため。遡りきれない（走査の上限を超えた・git が失敗した）ときは error にする（fail-closed）。

- status が `Conflict` の文書は、stale でも失敗にせず「解消待ち」として別に報告する（上の「status の 3 段階」のとおり sha を蒸留前に保つので、stale のまま留まるのが正しい状態）。解消待ちの件数は 0 件でも必ず出る
- shallow clone で履歴が足りないと判定できないので、exit 2 で止まる。CI では全履歴を取得する（GitHub Actions なら `fetch-depth: 0`）

### sha の追従

蒸留して本文を直したら、`distilled_from_sha` をそのコミットへ進める。コミットは自分自身の sha を含められないので、運用は「本文のコミット → sha 追従のコミット」の 2 コミットになる。

```bash
python3 .claude/scripts/hve/bump-distilled-sha.py knowledge/xxx.md   # 指定した文書を HEAD へ
python3 .claude/scripts/hve/bump-distilled-sha.py --all-stale --dry-run  # stale な文書を一覧
```

- bump は `distilled_from_sha` だけを書き換え、`updated` は触らない（上の「`updated` の規則」）
- `Conflict` の文書は bump しない
- **squash・rebase の後の追従**: squash や rebase で、sha が指していたコミットが HEAD の履歴から外れる（手元では通ることがある（その PR で source が変わっていないとき）が、新しく clone すると sha を解決できない）。`bump-distilled-sha.py --follow-rewritten <base>` は、その PR で変えた文書のうち、sha が HEAD から辿れず、旧 sha と HEAD で sources の中身が同じものだけを HEAD へ追従させる。旧 sha のあとに source が変わった文書は追従せずに報告する（蒸留し直す）。create-pr は squash の後に、review-pr は rebase の後の push の前にこれを実行して追従コミットを積む（ADR 0014）。push 済みのブランチでは rebase のたびに追従コミットが 1 つ増える。手で squash・rebase したときも同じように実行する
- **PR は merge commit でマージする**: GitHub の squash merge や rebase merge でマージすると、追従させた sha（PR ブランチのコミット）が main から辿れなくなり、以後の CI の stale 検査が「distilled_from_sha を解決できない」で落ち続ける。knowledge 文書を持つリポジトリでは、squash merge と rebase merge を無効にしておく

### 検査スクリプト

これらの検査の目的は、善意の誤操作（うっかりした書き換え・蒸留漏れ・リポジトリの破損）の検出であり、悪意ある改ざんの防御ではない。悪意のある作成者は同じ PR で検査スクリプトや CI の設定を書き換えられ、stale 検査も本物のコミットの sha を書けば黙らせられる。改ざんへの備えは PR のレビューと branch protection に任せる（ADR 0014）。


| スクリプト | 役割 | 終了コード |
|---|---|---|
| `check-knowledge.py` | frontmatter の必須項目、sources の実在（リポジトリ相対の正規形・大文字小文字まで一致）、stale 判定、解消待ちの報告 | 0 正常 / 1 違反 / 2 判定不能 |
| `bump-distilled-sha.py` | stale な文書の `distilled_from_sha` を追従させる | 0 / 1 / 2 |
| `check-decision-log.py` | 決定ログの不変性と ADR の番号の重複（artifact-management の「決定ログの不変性」） | 0 / 1 / 2 |
| `hooks/session-stale-check.sh` | Claude Code の SessionStart で stale を報告する | 常に 0 |
| `hooks/check-knowledge-impact.py` | Claude Code の PostToolUse で、source を編集したら影響する knowledge 文書を、knowledge を直接編集したら SoT の注意を出す | 常に 0 |

`check-knowledge.py` の主な引数（既定は本標準）:

- `--dir knowledge`（再帰）、`--exclude`（既定 `adr/` と `README.md`）
- `--required title,status,kind,sources,distilled_from_sha,updated`
- `--allow-empty-sources-with-decision-log`: `## 決定ログ` 節を持つ文書に限り、sources と `distilled_from_sha` が揃って空でもよい（stale 判定しない）。既定では sources は必須
- `--warn-only`: 違反があっても exit 0

**パース契約**: `bump-distilled-sha.py` は `check-knowledge.py` の STALE 行（`✗ <文書>: STALE ← <source> が distilled_from_sha(<値>) より後に更新されている（<7桁>）。…`）をパースする。SessionStart の hook は bump の出力の「STALE な文書は無い」と「（dry-run）<文書> → <sha>」を、create-pr（Step 6.1）は `--follow-rewritten` の出力の「✓ <文書>: <旧 sha> → <新 sha>」をパースする。書式を変えるときは呼び出し側も直す。create-pr・review-pr は sync で常に最新になるが、導入先の `.claude/scripts/hve/` は最後に setup.sh を実行した時点の写しなので、書式や引数を変えたら導入先に setup.sh の再適用を案内する。

### 組み込み方

配線は導入先が行う（setup.sh は配置だけ）。

- **CI**: pull request で、全履歴を取得して実行する（main への push で実行すると base と head が同じになり、決定ログの検査は差分なしで通る。main への直 push は branching で禁じている）

  ```yaml
  - uses: actions/checkout@<sha>
    with:
      fetch-depth: 0
  - run: python3 .claude/scripts/hve/check-knowledge.py
  - run: python3 .claude/scripts/hve/check-decision-log.py --base "origin/$BASE_REF" --head HEAD
    env:
      BASE_REF: ${{ github.base_ref }}
  ```

  `knowledge/` がまだ無いリポジトリでは、どちらの検査も対象 0 本で通る

  並行ブランチが同じ ADR の番号を取ったときの重複は、`--head` に pull_request の merge commit（checkout の既定）を渡したときだけ出る。checkout に `ref: ${{ github.head_ref }}` を指定すると出ない。チェックが通ったあとに別の PR が main に入ってもすり抜けないよう、branch protection でこの job を必須チェックにし、`strict`（Require branches to be up to date）を有効にする（hve-playbook 自身の設定は ADR 0027）

- **pre-push**: push するコミットを stdin から受けて検査する。例は hve-playbook の `.githooks/pre-push`。`git config core.hooksPath <dir>` で有効にする
- **Claude Code の hook**: `.claude/settings.json` に絶対パスで書く

  ```json
  {
    "hooks": {
      "SessionStart": [{"hooks": [{"type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR\"/.claude/scripts/hve/hooks/session-stale-check.sh", "timeout": 30}]}],
      "PostToolUse": [{"matcher": "Write|Edit", "hooks": [{"type": "command", "command": "python3 \"$CLAUDE_PROJECT_DIR\"/.claude/scripts/hve/hooks/check-knowledge-impact.py", "timeout": 10}]}]
    }
  }
  ```

  `check-knowledge.py` に引数（`--required`・`--allow-empty-sources-with-decision-log` など）を渡している導入先は、SessionStart の hook にも同じ引数を渡す（例: `bash "$CLAUDE_PROJECT_DIR"/.claude/scripts/hve/hooks/session-stale-check.sh --required status,kind,sources,distilled_from_sha,updated --allow-empty-sources-with-decision-log`）

SessionStart の hook は全文書の履歴を遡るので、大きなリポジトリでは数十秒かかることがある。セッションの開始が遅くなるなら SessionStart には配線せず、CI と pre-push に任せる。

AKM スキル（`.claude/skills/hve-akm/SKILL.md`）の Step 1・Step 3 はこれらのスクリプトを使う。

## doc_class（文書分類）

文書を役割で分類し、カバレッジ（どの役割の文書が揃っているか）を測る仕組み。

- 分類体系はプロジェクトが独自に定義する
- 各文書の frontmatter `doc_class` フィールドに分類コードを記載する
- AKM スキルの Step 4（カバレッジ分析）で、分類ごとの充足状況を検査する
