---
description: プロジェクト固有の補足 — setup.sh の再適用で消えない hve-local の置き場所・書式・優先順位。
---

# プロジェクト固有の補足（hve-local）

`setup.sh` は `.claude/rules/hve/`・`.claude/skills/hve-*/`・`.claude/agents/hve-*.md`・`.claude/workflows/hve-*.js`・`.claude/scripts/hve/` を削除してから置き直す。これらの写しに書き足した注記は、再適用のたびに消える。プロジェクト固有の補足・上書きは、写しではなく `.claude/rules/hve-local/` に書く。setup.sh はこのディレクトリに触れない。

## 置き場所

| 補足する対象 | ファイル |
|---|---|
| `rules/hve/<name>.md` | `.claude/rules/hve-local/<name>.md`（例: `hve-local/implement-flow.md`） |
| `skills/hve-*/SKILL.md`（`<skill 名>` は `hve-akm` など） | `.claude/rules/hve-local/<skill 名>.md`（例: `hve-local/hve-akm.md`） |

- 写し（`.claude/rules/hve/`・`.claude/skills/hve-*/`・`.claude/agents/hve-*.md`・`.claude/workflows/hve-*.js`・`.claude/scripts/hve/`）は編集しない。hve-playbook 本体では rules/hve・skills/hve-*・hve-scripts が原本なので、直接編集する。検査スクリプトの設定は引数で渡し、渡し方（CI・pre-push・hook の呼び出し）は hve-local に書く
- hve-local のファイルに `paths:` frontmatter を付けない。付けると対象ファイルを読んだときだけ読み込まれ、補足が効かない場面が出る

## 書式と優先順位

節の見出しで、標準のどの節をどう扱うかを示す。

- `## <対象> の「<節名>」を上書き`: その節は hve-local の記述に従い、標準の該当節は読まない
- `## <対象> の「<節名>」を補足`: 標準の該当節と併せて読む
- 見出しの形に当てはまらない記述は「補足」として扱う
- `<対象>` は拡張子を除いたファイル名か skill 名、`<節名>` は標準の見出しの文言を書く。上書きは配下の小見出しを含む
- 上書きは必要最小の見出しで行う。配下で標準のまま残したい小見出しがあれば、hve-local 側に「標準の『…』に従う」と書く

標準と hve-local が食い違ったときに hve-local が優先するのは、「上書き」と明示した節だけ。上書きが及ぶのは指定した節の中身だけで、rule 同士の優先順位（捏造禁止を最優先とするなど）は変えない。local-overrides.md 自身は上書き・補足の対象にしない。この優先順位は hve-local と標準の間の話で、導入先の CLAUDE.md の記述の効力は変えない。

## 運用

- hve-* skill を使うときは、`.claude/rules/hve-local/<skill 名>.md` の該当節を skill の手順と突き合わせてから進める（skill は呼び出したときに読み込まれるので、後から読んだ skill の本文で上書き節を見落とさないようにする）
- hve-playbook を再適用して標準が変わったら、hve-local の見出しが指す節が標準に実在し、「上書き」節が標準の変更に追従しているかを見直す
