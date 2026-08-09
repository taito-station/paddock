# 0073. ADR を一次資料層へ統合し、D01〜D24 文書クラスと機械検査を導入する

## ステータス

承認済み（PR1 で ADR 移動を実装。D クラス体系・機械検査・プロダクト目標は後続 PR）。

## コンテキスト

paddock は HVE（dahatake/HypervelocityEngineering, MIT）の 3 層蒸留モデル（original-docs → qa → knowledge）と mdq 検索を既に取り込んでいる。しかし実測すると、層の切り方と運用の両方に構造的な問題があった。

### 層の重複が実害を出している

#568 の 4 点セット（original-docs / qa / knowledge / ADR、合計 515 行）を全文照合した結果:

- `docs/knowledge/monitor-loop-sleep-resilience.md` の本文 103 行のうち **88 行（85%）が ADR 0072 と 1:1 対応**し、knowledge が追加した決定は **0 件**。固有の価値は運用者向けの読み方 6 行に集約されていた。
- 「5 秒刻みの根拠（DarkWake 累計 28 秒）」「単調時計で所要を測る理由」「JST 変換を持ち込まない理由」は、いずれも **qa / knowledge / ADR の 3 箇所に語順までほぼ同一**で存在した。
- `docs/qa/QA-analyze-384.md` の Q2/Q3 は回答文が **約 90% 逐語**で knowledge へ移送され、knowledge が足した固有情報は 1 行だった。
- `docs/original-docs/` 4 本はすべて **GitHub Issue 本文の 25〜38% を逐語コピー**していた。しかも #384 は「別 issue」→「#379・実装済」に改変、#389 は「現状」章を削除、#401 は「要件」章 4 項目を削除しており、**原本として機能していない**。
- 同一の実測（`name=カップ` → starts=0）が **5 ファイル**に重複して存在した。

### 蒸留層の権威が逆転している

`docs/qa/QA-setup-boilerplate-410.md` には「【追記・#453 で覆る】`NoopParser` / `NoopFetcher` スタブは削除された」とある。ところが蒸留先の `docs/knowledge/app-bootstrap.md` は `status: Confirmed` のまま `NoopParser` の注入を推奨し続けている。コードを実測すると `NoopParser` はソースツリーに **1 件も存在しない**。

「qa は生ファイル、knowledge が確定知」という規約（[docs/qa/README.md](../qa/README.md)）と実態が逆転しており、**knowledge を信じると存在しない API を書く**。`docs/knowledge/README.md` の第 6 ステップ「sources 追従」は規約として存在するが、機械検査が無いため守られていない。

### 蒸留が日常開発に乗っていない

knowledge / specifications 22 本の `updated` は全件 2026-07-16〜07-30 に集中し、`distilled_from_sha` も 11 本が同一 SHA。一括整備で作られたきり止まっている。`status` は全 22 本が `Confirmed` で、`Tentative` / `Conflict` の運用実績が無い。

### 分類軸が無い

`docs/` にプロダクトの目標・成功条件・非目標を書いた文書は **0 件**（全 106 本を検索して該当なし）。方向性は ADR 71 本を読み解くことでしか復元できない。また文書を横断的に分類する軸が無く、`docs/adr` / `docs/specifications` / `docs/knowledge` というディレクトリの区別しか無かった。

## 決定

### 1. ADR を `docs/original-docs/` へ物理移動し、一次資料層に統合する

ADR 71 本（0001〜0071）を `docs/adr/` から `docs/original-docs/` へ移す。ディレクトリ `docs/adr/` は廃止する。

命名で 2 系統を分離する。**この規約が ADR 番号重複検出の判定根拠**になる。

| 種別 | 命名 | 例 |
|---|---|---|
| ADR | **0 埋め 4 桁**（0001〜0999） | `0055-ev-layer-separation-circular-break.md` |
| issue 由来の一次資料 | **issue 番号・0 埋めしない** | `382-live-server-now.md` |

`scripts/check-adr-numbers.sh` は走査先を `docs/original-docs` に変え、先頭 1 文字が `0` かで ADR を分離する。非 ADR は**黙ってスキップ**する（警告に載せると本来見るべき重複検出が埋もれる）。ただし 0 埋めを忘れた ADR を取りこぼすと重複検出が静かに無効化されるため、H1 が ADR 書式（`# ADR 0001: …` / `# 0071. …` の 2 系統）に見えるファイルは**致命（exit 1）**で拾う。ADR 0 件も、従来の `exit 0`（fail-open）から `exit 1` へ変える。

### 2. ADR の内容は knowledge へ全部写す。同期は機械検査で担保する

読む入口を knowledge に一本化する。ADR の決定・理由・却下案・影響を knowledge に写し、ADR 自体は一次資料として不変のまま残す。

重複を許す代わりに、`sources` に列挙されたファイルの最終コミットが `distilled_from_sha` の子孫かを機械検査する（`git merge-base --is-ancestor`）。CI と pre-push の両方に配線する。

### 3. HVE の D01〜D21 文書クラスを採用し、D22〜D24 を追加する

D01〜D21 は番号・名称を変えず採用する（HVE との語彙互換を保ち、将来の追加移植の摩擦を避ける）。paddock 資産の実測で、**99 本中 54 本（54.5%）が D01〜D21 のどこにも入らない**ことが分かったため、3 クラスを追加する。

| クラス | 内容 | 該当本数 |
|---|---|---|
| D22 | 予測モデル・特徴量仕様 | 31 |
| D23 | 買い方・資金配分ルール | 18 |
| D24 | 実験・検証記録／棄却証跡 | 5 + `-rejected` ADR 24 本 |

**D03 / D12 / D13 / D14 / D20 は「N/A（単独開発・ローカル運用）」を 1 行宣言して閉じる**。空文書は作らない。

物理表現は frontmatter `doc_class`（正本）とし、`tags` に同値をミラーする。ファイル名は変えない。

### 4. プロダクト目標文書（D01）を新設する

「数値で競馬を見る」「買い方を楽しく売れる形で提示する」を目標として明文化し、成功条件（ROI ≥ 100% ゲート・精度実績）と非目標（棄却 ADR 24 本から復元）を 1 枚にまとめる。**収益化の具体（価格・販路）は書かない**。

## 理由

- **ADR と一次資料は「一度置いたら書き換えない（RO）」という性質が同じ**。同じ層に置くことで、`ADR → knowledge` の写しが例外的な重複ではなく規約どおりの蒸留になる。層の数を減らさずに、責務の説明を一本化できる。
- **移動コストが小さいことを実測で確認した**。`docs/adr` と `docs/original-docs` は同じ階層深さ（`docs/` 直下）なので、相対リンクはどの参照元からも「`adr` → `original-docs`」の 1 語置換で閉じる。ADR 本文が持つ兄弟相対リンク 8 件・`../specifications/` 6 件・`../images/` 1 件・`../../deployments/` 2 件は**無変更で解決する**。ファイル名衝突も 0 件だった。
- **「全部写す」を選ぶ以上、機械検査は必須**。写した量に比例して stale 面積が増える。`app-bootstrap.md` の `NoopParser` 事故は 1 件で済んだが、71 ADR ぶんに広げれば人手の規律では守れない。ADR 番号の重複検出（#254）と同じ判断——人手で再発が防げないものは機械で弾く。
- **D01〜D21 をそのまま採るのは HVE 互換のため**。番号を変えると将来 HVE の資産（skill・prompt）を追加移植するときに読み替えが要る。空クラスが 12 個出るが、うち 5 個は N/A 宣言で閉じ、残り 4 個（D01 成功条件・D07 用語集・D15 SLO/Runbook・D21 CI/CD の文書化）は**真の欠落**で、埋めること自体が D 体系採用の実利になる。
- **企業分析・業界分析が無くても上流は書ける**。HVE の ARD ワークフローは `target_business` 指定時に Step 1（事業分野候補列挙）を skip する設計を持っており、対象が決まっている個人プロダクトは正規ルートでその経路に乗る。paddock の「業界分析」に相当する市場（オッズ）の性質分析は、ADR 0027 / 0055 / 0058 / 0059 / 0067 として**既に蓄積済み**で、足りないのは上位のゴール文書 1 枚だった。

## 却下した代替案

- **ADR を `docs/adr/` に残し、位置づけの宣言だけ変える**。リンク破壊もツール改修もゼロで済み、mdq は `docs/adr` を索引済みなので検索体験も変わらない。実利/コスト比では最も良いが、ディレクトリ構成が 3 層モデルと一致しないままになる。**利用者の判断で物理移動を採用した**。
- **`docs/original-docs/adr/` へサブディレクトリとして移動**。生ログと ADR の混在を避けられるが、パス一斉改修のコストは同じで、階層深さが変わるぶん ADR 本文の相対リンク 17 件も書き換えが要る（フラット移動なら不要）。
- **knowledge を「複数 ADR を横断するときだけ作る」に限定する**（＝ ADR 1 本に knowledge を作らない）。#568 の 85% 重複は消えるが、「今どうなっているか」を知るのに ADR と knowledge を往復することになる。読む入口の一本化を優先して却下した。
- **D22〜D24 を作らず D06（業務ルール・判定表）/ D17（UAT）へ押し込む**。HVE と完全同一の 21 クラスを維持できるが、D06 の必須項目「判定表・override 承認者・発効/失効日・根拠規程」が予測モデル 31 本すべてで UNKNOWN になる。統計モデルに承認者も規程根拠も存在しない。
- **D クラスをファイル名プレフィックス（`D08-*.md`）で表現する**（HVE 流）。`mdq --paths` で絞れる利点があるが、22 本のリネームで `sources` 参照が再度壊れる。`doc_class` + `tags` ミラーで同等の絞り込みが得られるため却下。

## 影響

- **移動**: ADR 71 本が `docs/adr/` → `docs/original-docs/`。`docs/adr/` は消滅。
- **変更（機械置換 187 箇所 / 33 ファイル）**: frontmatter `sources` のパス、本文の相対リンク、規約文。`git grep` / `git ls-files` に限定して実施した（`.claude/worktrees/` の並走 worktree 3 本がそれぞれ完全な `docs/adr/` を持つため、`grep -r` では別ブランチの作業コピーを破壊する）。
- **変更**: `scripts/check-adr-numbers.sh`（走査先・ADR 分離・fail-closed 化）、`mdq.toml`（`docs/adr` root 削除。`iter_markdown` は roots を重複除去しないため、併記すると同一ファイルを 2 回索引しに行く）。
- **不変**: ADR 本文（71 本すべてバイト同一で移動）、ADR の採番方式、CI ジョブ ID `adr`（ruleset #461 の必須チェックなので改名しない）。
- **運用**: 新しい ADR は `docs/original-docs/0NNN-*.md` に置く（採番は `scripts/check-adr-numbers.sh next`）。issue 由来の一次資料は 0 埋めしない。mdq で ADR だけに絞るなら `--paths "docs/original-docs/0*"`。
- **後続**: D クラス体系と機械検査（PR2）、プロダクト目標と REQ-ID 規約（PR3）、質問票 skill の汎用改修（PR4・dotclaude 側）。既存 71 ADR の REQ-ID 遡及紐付けと knowledge への全写しの実施は段階的に進める。
- 関連: #254（ADR 番号重複検出）／ADR 0064（second source を戒める）／[docs/knowledge/README.md](../knowledge/README.md)（蒸留規約の正）。

## 再現方法

```sh
# ADR の重複検出（71 件・次番号）
bash scripts/check-adr-numbers.sh

# 0 埋めを忘れた ADR が致命として拾われること（fail-closed の実証）
mv docs/original-docs/0071-topcoat-framework-evaluation-rejected.md docs/original-docs/71-topcoat.md
bash scripts/check-adr-numbers.sh   # → ✗ … exit 1
git checkout -- docs/original-docs/

# 旧パスの残存が無いこと（履歴参照コメント 2 件を除く）
git grep -n 'docs/adr' -- .
git grep -n '\.\./adr/' -- .

# mdq 再索引と ADR 絞り込み
scripts/mdq index
scripts/mdq search --q "EV 層分離" --paths "docs/original-docs/0*" --top-k 3
```
