# original-docs — 読み取り専用の一次資料（生素材 + ADR）

knowledge を蒸留する**元になる資料**を置く場所。**ここのファイルは書き換えない**
（HVE の original-docs と同じ思想。source は改変せず、蒸留は knowledge 側で行う）。
全体像は [docs/knowledge/README.md](../knowledge/README.md)。

> 例外は 2 つだけ。(1) ディレクトリ移設に伴うパス表記の是正（実績は ADR 0073 の 1 件——
> `0062-workout-cyokyo-feature-rejected.md` の本文中にあった `docs/adr/0061` の表記）。
> (2) 下記の `<!-- not-an-adr -->` マーカーの付与（判定用メタデータであって本文ではない）。
> 内容の訂正・追記はしない——古い記述は「その時点で何を知っていたか」の記録として残す。

ADR 0073 で ADR をここへ統合した。ADR も生素材も「一度置いたら書き換えない（RO）」という
性質が同じで、`ADR → knowledge` の写しを規約どおりの蒸留として扱えるため。

## ファイル命名（2 系統・混同しない）

`scripts/check-adr-numbers.sh` はこの規約でファイルを分離する。破ると ADR の番号重複検出が
静かに無効化されるため、**新しいファイルを置くときは必ず従う**。

| 種別 | 命名 | 例 |
|---|---|---|
| **ADR**（決定記録） | **0 埋め 4 桁** + kebab（`0001`〜`0999`） | `0055-ev-layer-separation-circular-break.md` |
| **issue 由来の一次資料** | **GitHub issue 番号（0 埋めしない）** + kebab | `382-live-server-now.md` |

判定は「`0` + 3 桁で始まるか」（`^0[0-9]{3}`）。issue 番号は 0 埋めしないので両者は排他に分かれる。
**上限は 0999**——4 桁を超える採番が必要になったら、この規約と `check-adr-numbers.sh` の判定を
併せて見直す（今の実装では `1000-*.md` は「ADR に見えるのに 0 埋め 4 桁でない」として弾かれる）。
ADR の採番は `scripts/check-adr-numbers.sh next`（並行 clone / worktree での二重採番を機械検出する）。

**ADR は必ずこのディレクトリの直下にフラットに置く。** サブディレクトリを切ると重複検出と採番の
両方から不可視になるため、`check-adr-numbers.sh` が階層配置を致命として弾く。

### `<!-- not-an-adr -->` — 一次資料が ADR と誤判定されるときの逃げ道

`check-adr-numbers.sh` は「0 埋めを忘れた ADR」を取りこぼさないため、ADR 名でないファイルも
本文構造（コードフェンスの外の行頭に `## ステータス` と `## 決定` が同居）で判定する。

ここは issue 本文や外部資料を**逐語転記**する層なので、ADR 草案を含む issue をそのまま置くと
この判定に引っかかり、CI と pre-push が落ちる。転記内容は書き換えない（RO）ので、その場合は
**ファイル先頭 10 行以内に次の 1 行を置く**:

```markdown
<!-- not-an-adr -->
```

これは本文ではなく判定用のメタデータなので、RO 原則の例外として認める。**ADR を隠すために
使ってはならない**——本物の ADR は 0 埋め 4 桁の名前で直下に置く。

## 何を置くか

- **ADR（決定記録）**。決定・理由・却下した代替案・影響。棄却した案も同じ厚みで記録する
- netkeiba / JRA の挙動メモ（エンコーディング・ページ生成タイミング・DOM 構造など外部仕様の観察）
- 外部から持ち込んだ資料・仕様の写し
- 生のログ（`pmset -g log` の抜粋・実行時 stdout・バックテストの生出力など）。**導出値だけを
  knowledge に書くと後から別の仮説を検証できなくなるため、素材そのものを残す**
- 調査時点のコード所見（対象 SHA を明記する）

## 何を置かないか

- 確定した運用ルール・ドメイン知 → `docs/knowledge/` or `docs/specifications/`
- 質問票と回答 → `docs/qa/`
- コード・設定（リポジトリ本体で管理）

## 運用

1. 一次資料をここに置く（RO）。決定を伴うものは ADR として起票する。
2. Claude が読んで欠落/不整合を検出し、`docs/qa/` に質問票を起票。
3. 回答済み qa と ADR を knowledge に差分マージ。**ADR の内容は knowledge へ全部写す**
   （読む入口を knowledge に一本化する）。original-docs 自体は残す（トレーサビリティ）。
   **※ 既存 ADR 72 本の一括写しは stale 機械検査の配線が完了するまで開始しない**
   （ADR 0073 / [docs/knowledge/README.md](../knowledge/README.md) の移行中ブロック参照）。
   それまでは knowledge だけでなく ADR 原本も読む運用。

> 一次資料は mdq の索引対象（`mdq.toml`）に含まれるので、`scripts/mdq search` で横断検索できる。
> ADR だけに絞るなら `--paths "docs/original-docs/0*"`。
