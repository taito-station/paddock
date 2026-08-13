---
status: Confirmed
kind: knowledge
sources:
  - docs/original-docs/0073-adr-into-original-docs-and-doc-classes.md
  - docs/original-docs/0082-sources-coverage-checks.md
  - docs/qa/QA-sources-coverage-checks-596.md
distilled_from_sha: "722d987"
updated: "2026-08-13"
---

# 文書クラス D01〜D24 レジストリ

HVE（[dahatake/HypervelocityEngineering](https://github.com/dahatake/HypervelocityEngineering), MIT）の
文書クラス **D01〜D21 を番号・名称を変えずに採用**し、競馬予想ドメイン固有の **D22〜D24 を追加**した
もの（ADR 0073 決定 3）。番号を変えないのは、将来 HVE の資産を追加移植するときに読み替えを
発生させないため。

各文書は frontmatter の `doc_class` で自分のクラスを宣言する。**このファイルがクラス定義の正本**で、
`scripts/check-doc-classes.py` が本ファイルと全文書の宣言の整合を機械検査する。

> このファイル自身には `doc_class` を付けない（クラス定義そのものであってクラス付き文書ではない）。

## frontmatter の書き方

```yaml
doc_class: [D22, D24]   # 正本。第 1 要素が主クラス（その文書の中心的な関心事）
tags: [D22, D24]        # mdq 用ミラー。doc_class と完全一致させる（checker が強制）
```

- **常にリスト**（単一クラスでも `[D19]`）。単一値とリストの両方を許すと、消費側すべてで型分岐が
  必要になる。
- **フロースタイル 1 行**で書く。checker を stdlib のみで実装するため（`^doc_class:\s*\[…\]` の
  1 正規表現でパースできる）。`sources` は既存のブロックスタイルのまま。
- ID は `D` + 2 桁ゼロ埋め。`tools/mdq/mdq/query_router.py` の ID パターンが `D\d{2}` を要求する
  ため `D1` は不可。
- **`tags` へのミラーは mdq のため**。mdq は frontmatter を検索に使わず、絞り込めるのは `tags` だけ
  （`tools/mdq/mdq/indexer.py`）。`doc_class` 単独では mdq から見えないので同値を `tags` に置く。
  二重管理の drift は checker が防ぐ。

```sh
scripts/mdq search --q "EV ゲート" --tags D23 --top-k 5   # クラスで絞り込む
```

## クラス一覧

`active` = このリポジトリで運用する / `n/a` = 適用外を宣言して閉じる。
「現行」列は `doc_class` に当該クラスを含む文書数で、**checker が実態と突き合わせる**（手書きとの
不一致は CI で落ちる）。`0` は充足ギャップ（active のみ warning）。

> 表の書式は checker がパースする契約でもある。`| D01 | 名称 | active | 0 |` の 4 列を崩さない
> （崩れた行は「書式が崩れている」として error にする——黙って落とすとそのクラスが「未定義」に
> なり、参照している文書が全部 error になって原因が読めなくなるため）。

<!-- doc-classes:begin -->
| クラス | 名称 | 状態 | 現行 |
|---|---|---|---|
| D01 | 事業意図・成功条件定義書 | active | 1 |
| D02 | スコープ・対象境界定義書 | active | 1 |
| D03 | ステークホルダー・承認権限・責任分担表 | n/a | 0 |
| D04 | 業務プロセス仕様書 | active | 1 |
| D05 | ユースケース・シナリオカタログ | active | 0 |
| D06 | 業務ルール・判定表仕様書 | active | 1 |
| D07 | 用語集・ドメインモデル定義書 | active | 1 |
| D08 | データモデル・SoR/SoT・データ品質仕様書 | active | 6 |
| D09 | システムコンテキスト・責任境界・再利用方針書 | active | 2 |
| D10 | API / Event / File 連携契約パック | active | 11 |
| D11 | 画面・UX・操作意味仕様書 | active | 7 |
| D12 | 権限・認可・職務分掌設計書 | n/a | 0 |
| D13 | セキュリティ・プライバシー・監査・法規マトリクス | n/a | 0 |
| D14 | 国際化・地域差分仕様書 | n/a | 0 |
| D15 | 非機能・運用・監視・DR 仕様書 | active | 3 |
| D16 | 移行・導入・ロールアウト計画書 | active | 0 |
| D17 | 品質保証・UAT・受入パッケージ | active | 2 |
| D18 | Prompt ガバナンス・入力統制パック | active | 0 |
| D19 | ソフトウェアアーキテクチャ・ADR パック | active | 11 |
| D20 | セキュア設計・実装ガードレール | n/a | 0 |
| D21 | CI/CD・ビルド・リリース・供給網管理仕様書 | active | 1 |
| D22 | 予測モデル・特徴量仕様 | active | 6 |
| D23 | 買い方・資金配分ルール | active | 3 |
| D24 | 実験・検証記録／棄却証跡 | active | 5 |
<!-- doc-classes:end -->

## D22〜D24（paddock 固有の追加）

D01〜D21 は企業の業務システムを要求定義する体系なので、paddock の資産の過半（予測モデル・買い方・
検証記録）を受ける器が無い。無理に既存クラスへ押し込むと必須項目が総 UNKNOWN になるため追加した
（ADR 0073 の却下案参照。例: 予測モデルを D06「業務ルール・判定表」に入れると「override 承認者」
「発効日」「根拠規程」がすべて空欄になる——統計モデルに承認者も規程根拠も存在しない）。

| クラス | 名称 | 扱う内容 |
|---|---|---|
| **D22** | 予測モデル・特徴量仕様 | 素性定義・重み・縮約/較正パラメータ・確率推定の手順・resolution/calibration 指標 |
| **D23** | 買い方・資金配分ルール | 券種選択・EV/ROI 閾値・ポートフォリオ構成・予算配分・軸ロック運用 |
| **D24** | 実験・検証記録／棄却証跡 | backtest / ハーネスの設計と実測、採らなかった案とその根拠の集約 |

**D22 と D23 を分けているのは意図的**。ADR 0055 が確立した「確率と買い方の分離」（順位付けは
blended 確率、EV は純モデル確率 × 市場オッズ）を、文書クラスの階層でも表現する。

## N/A 宣言（D03 / D12 / D13 / D14 / D20）

適用外を**明示的に閉じる**。「まだ書いていない」と「そもそも要らない」を区別しないと、充足ギャップの
警告が恒久的なノイズになり、本当の欠落が埋もれる（D07 用語集がその実例で、#598 で解消した）。

<!-- doc-classes-na:begin -->
| クラス | N/A の理由 | 再開条件 |
|---|---|---|
| D03 | 単独開発・単独運用者。承認権限も責任分担も発生しない | 複数人で開発・運用するようになったとき |
| D12 | 認証認可を持たない。API はローカル/LAN 前提で、ADR 0022 が置いたのは no-op の差し込み口のみ | 外部公開して利用者を識別する必要が出たとき |
| D13 | 個人利用のローカル運用。PII を扱わず、監査証跡・法規要件の対象にならない | 予想を第三者へ提供し、対価や個人情報が絡むようになったとき |
| D14 | JRA 専用・日本国内・JST 固定。通貨も日本円のみ | 海外競馬を扱う、または複数ロケールへ提供するとき |
| D20 | 外部入力を受けない単独プロセス群。脅威モデルを立てる対象境界が存在しない | ネットワーク越しに未信頼の入力を受けるようになったとき |
<!-- doc-classes-na:end -->

**N/A クラスを `doc_class` に指定した文書があれば checker はエラーにする**（宣言と実態の矛盾を
放置しない）。解除するときは本表から行を削除し、上の一覧の状態を `active` に変える。

## 充足ギャップ（active だが 0 本）

D 体系を採用する最大の実利は、**書くべきなのに無い文書が見えるようになること**。現時点のギャップ:

| クラス | 現状 | 対応 |
|---|---|---|
| **D05** ユースケース | `README.md` の「何ができるか」が最も近いが、UC カタログの形では無い | 優先度低 |
| **D16** 移行・導入 | ADR 0070（DB マイグレーション運用）が近いが移行計画書ではない | 優先度低 |
| **D18** Prompt ガバナンス | 実質は [`docs/knowledge/README.md`](README.md)（3 層モデル・SoT の優先順位）と `CLAUDE.md` が担っている | 名前だけ空。当面このままでよい |

## 体系側の既知の穴

**D07 は用語集だけで、ドメインモデル定義書が無い。** [glossary.md](glossary.md)（#598）が満たしたのは
クラス名の前半（用語集）で、後半——**エンティティ相互の関係を横断で示した文書**——は D07 にも D08 にも
無い（個別の構造は各仕様書が持つ。例: `race_id` の 12 桁構成は
[netkeiba-datasource.md](../specifications/netkeiba-datasource.md) の「race_id 構築規則」）。**「現行 1」で充足ギャップの warning は消えるので、
機械検査はこの穴を報せない**。埋めるかどうかは未決（現状 6 本の D08 文書が個別のデータ仕様を
持っており、横断のモデル図が要る局面がまだ出ていない）。用語集そのものの収録範囲にも未着手の領域がある（[glossary.md](glossary.md)
「収録と参照の基準」が正）。

**D23（買い方・資金配分ルール）の一次定義がリポジトリの `docs/` 配下に無い。** 現行ルールの本体は
プロジェクトルートの `CLAUDE.md`「買い方ルール」節にあり、`docs/` 側の D23 文書のうち
`betting-rule-history.md` / `live-ev-buy-view.md` は**根拠・棄却記録・画面契約**に留まるが、
`ev-kelly-bet-selection.md` は #594 で **D23 の REQ 表（要件と検証手段）** を持つようになった
——「何を守るか」と「どう測るか」は docs 側、「今どう張るか」の運用指示は `CLAUDE.md` 側、という分担。`CLAUDE.md` は毎セッション読まれる運用指示なので現状で機能して
いるが、「クラスの主文書がクラス体系の外にある」状態ではある。

D01（[product-goals.md](product-goals.md)）を作った時点では**移していない**（検討経緯は #582）。
買い方ルールは毎セッション自動で読まれることが実効性の source で、`docs/` へ移すと「読まれる保証」を
失う代わりに得られるのが `doc_class` 付与だけになる、という理由。現状は D01 側が買い方の**目標と
非目標**（REQ-D01-001/003/007・非目標 C 群）を持ち、`CLAUDE.md` が**現行ルールの運用指示**を持つ。
**移すかどうかを決めるのは ADR の仕事**なので、この段落は決定記録ではなく現状の説明として読むこと。

## 割当の一覧

各文書の `doc_class` は **frontmatter が正**で、この表は読みやすさのための索引。

> **この索引表は checker が frontmatter と 1 対 1 で突き合わせる**（#604）。行の欠落・余剰・値の
> 不一致（順序違いを含む）はいずれも error。上の一覧の**クラス別集計数**だけでは、主クラスの
> 順序入替や 2 文書間のクラス交換が検出できないため、索引側で塞いでいる。
> **knowledge / specifications を 1 本足す・消す・`doc_class` を変える**ときは、同じ PR でこの表も直す。
>
> 表の書式も checker がパースする契約。`| knowledge/glossary.md | [D07] |` の **2 列**で、
> 左は `docs/` を剥がした**素のパス**（リンク化しない）、右は `[D11, D10]`（`, ` 区切り・**順序込み**）。
> 崩れた行は「割当索引の書式が崩れている行がある」として error にする。
> 表の範囲はマーカー（`doc-classes-index`）で切り出すので、マーカーの内側に行を書く
> （マーカーを消すと検査が成立しないため、`--warn-only` でも落ちる）。

<!-- doc-classes-index:begin -->
| 文書 | doc_class |
|---|---|
| knowledge/analyze-search-and-state.md | [D11, D10] |
| knowledge/app-bootstrap.md | [D19, D15] |
| knowledge/ci-pipeline.md | [D21, D19, D17] |
| knowledge/glossary.md | [D07] |
| knowledge/live-freshness-calibration.md | [D11, D10] |
| knowledge/monitor-loop-sleep-resilience.md | [D15, D19] |
| knowledge/product-goals.md | [D01] |
| knowledge/race-card-display-metadata.md | [D08, D10, D11] |
| knowledge/scoring-factor-collection.md | [D22, D19] |
| specifications/backtest.md | [D24, D17, D19] |
| specifications/betting-rule-history.md | [D24, D23] |
| specifications/ev-kelly-bet-selection.md | [D23, D22] |
| specifications/feature-resolution-diagnosis.md | [D24, D22] |
| specifications/fetch-stage-split.md | [D19, D08, D15] |
| specifications/jockey-recent-form.md | [D22, D24] |
| specifications/learned-model-harness.md | [D24, D22, D19] |
| specifications/live-ev-buy-view.md | [D11, D23, D10] |
| specifications/netkeiba-datasource.md | [D10, D08, D09] |
| specifications/predict-session.md | [D11, D19, D08] |
| specifications/prediction-json.md | [D10, D08] |
| specifications/prediction-search-api.md | [D10, D08, D19] |
| specifications/probability-estimation.md | [D22, D19] |
| specifications/race-result-ingestion.md | [D04, D10, D11] |
| specifications/rest-api-read.md | [D10, D19, D09] |
| specifications/session-write-api.md | [D10, D06] |
| specifications/web-spa.md | [D11, D02, D10] |
<!-- doc-classes-index:end -->

## `sources` の網羅性検査（ADR 0082）

stale 検査（`sources` に挙げた一次資料への追従）の**前提**を守る 2 つの検査。どちらも
「`sources` の中身が正しいか」を見る側で、片方だけでは穴が残る。

### ADR の被参照（orphan 検査）

**ADR は必ずどこかの knowledge / specifications の `sources` から参照される。** これが
「ADR の内容は knowledge へ全部写す」（ADR 0073 決定 2）を機械で担保する前提になっている——
stale 検査が見るのは `sources` に**挙がっている**行だけなので、載せ忘れた ADR は更新されても
誰も気づかない。**`sources` から行を消せば stale も消える**、という網羅性の穴の片側。

checker は `docs/original-docs/` の **0 埋め 4 桁の ADR** を列挙し、全 knowledge /
specifications の `sources` の和集合に含まれないものを error にする（ADR 0082）。issue 由来の
一次資料（`382-...` のように 0 埋めしない番号）は蒸留先を持つとは限らないので対象外。
**判定は `scripts/check-adr-numbers.sh` の `^0[0-9]{3}` と同一の述語**——先頭 0 を落として
「4 桁」で判定すると、issue 番号が 4 桁に届いた時点で `1024-foo.md` が ADR と誤判定される。

**ADR を新設したら、同じ PR でどこかの `sources` に載せる。** 載せられない ADR は下の例外表に
**理由付きで**登録するのが正規の逃げ道（`--warn-only` は逃げ道に数えない）。**塞がるのは
「`sources` への登録を後続 PR に回すこと」まで**で、本文の写しを後回しにする運用は機械では
止まらない（下記「保証しないこと」）。

**正当な例外は 2 カテゴリ**:

1. **規約そのものを定めた ADR** で、確定知として写す先が無いもの（現行の 0074）
2. **supersede された ADR** で、下流が後継 ADR だけを参照するようになったもの。旧 ADR を
   `sources` から落とすのは正しい操作だが、落とした瞬間に orphan になる。**supersede する PR で
   例外表に「ADR NNNN に supersede された」と書く**（CLAUDE.md「決定を変えるときは新しい ADR で
   supersede する」と直交する必然）

> 表の書式も checker がパースする契約。**見出し行は `| ADR | 例外の理由 |`**（左セルの
> `ADR` で見出しと判定するので、列名を変えたり太字にすると見出し行が例外エントリとして
> 読まれる。割当索引の `文書` と同じ規約）。データ行は `| docs/original-docs/0074-....md | 理由 |`
> の **2 列**で、左は **`sources` と同じリポジトリルート相対パス**（上の割当索引は `docs/` を
> 剥がすが、**この表の比較相手は `sources` なので敢えて揃えていない**。正規化を 1 段挟むほど
> 「どちらの形式か」の取り違えが増える）。理由の空欄は error。
> パスは **`sources` と同じ正規形**（`./docs/...` のような非正規形は error。非正規形は
> 実在検査を通っても stale 判定の突合から静かに外れるため、形式を 1 つに強制している）。
> セル内の `|` は `\|` でエスケープする（素の `|` は列数不一致＝書式崩れ error。割当索引も同じ）。
> **例外表そのものも検査する**——実在しない ADR / 実在するが ADR ではないファイル / 実際は
> `sources` から参照されている（＝不要になった例外）/ 行の重複 / ADR 列の空 / 見出し行の
> 列名違い / 行の書式崩れはいずれも error。腐った例外を残すと、次に本物の orphan が出たとき
> 「例外表にあるから安心」と誤読される。
> マーカーを消すと検査が成立しないので `--warn-only` でも落ちる。**ADR が 0 件のとき**も
> 同じ扱い（判定条件の取り違えで検査が丸ごと無効化される fail-open を塞ぐ）。

<!-- adr-orphan-exceptions:begin -->
| ADR | 例外の理由 |
|---|---|
| docs/original-docs/0074-no-issue-body-transcription.md | 文書運用の規約そのものを定めた ADR で、蒸留先の knowledge を持たない。決定内容（issue 本文を転記しない）は `docs/original-docs/README.md` の「何を置かないか」と本ファイルの運用規約が実効しており、確定知として写す先が無い |
<!-- adr-orphan-exceptions:end -->

### REQ 表の出典も `sources` に載せる

同じ穴のもう片側。**REQ 表の `出典` 列が名指しした `docs/original-docs/` 配下のファイルは、
その文書の `sources` にも載っている**ことを checker が error で保証する（ADR 0082 決定 2）。
`出典` 列は「その要件の根拠」と定義された唯一の機械可読な場所なので、ここが指した一次資料だけは
必ず watch 対象に入る。

- **基準パスはリポジトリルート相対に正規化してから比較する。** `出典` 列は文書からの相対
  （`../original-docs/0055-....md`）、`sources` はルート相対で、揃えないと必ず食い違う。
- **対象は `docs/original-docs/` 配下全体**（4 桁 ADR に限らない。issue 由来の一次資料も蒸留元）。
- **対象外**: 外部 URL（GitHub issue 等。一次資料ファイルではないので `sources` に載せられない）/
  兄弟の knowledge・specifications へのリンク（蒸留元ではなく相互リンク）/ リンク切れ
  （本文リンク検査が別に報告する担当。ここで拾うと 1 本の切れリンクに 2 件の error が出るうえ、
  `sources` は実在ファイルしか受け付けないので「`sources` に足せ」が誤った助言になる）。

導入時の実測（main `5ae6466`）は **ADR 80 本中 orphan 1 本（0074 のみ）/ REQ 行 26・出典の
リポ内リンク 37 本中 未収載 0 件**。真であるうちに機械化した、というのが ADR 0082 の判断。

### この 2 検査が保証しないこと

**機械化できたのは「`sources` への登録」までで、「knowledge へ写したか」ではない。** ADR を
任意の文書の `sources` に 1 行足せば検査 12 は通る（[product-goals.md](product-goals.md) は
34 本の ADR を索引目的で `sources` に並べている）。**写しの中身——決定・理由・却下案・影響を
実際に書いたか——は人手の規律に残る。** ここを誇張して読むと「機械が見ているから大丈夫」という
誤った安心になる。

**参照元が複数ある ADR には、元の穴（`sources` から行を消せば stale も消える）が開いたまま。**
導入時点で **ADR 81 本のうち 48 本（59%）が 2 文書以上から参照**されている。これらは 1 つの文書の
`sources` から落としても検査 12 は鳴らず（他がまだ参照している）、その文書の REQ `出典` に
載っていなければ検査 11 も鳴らない。塞げたのは**「最後の 1 本を落とす」ケースと「REQ が根拠として
名指しした出典」だけ**で、中間は人手の規律に残る。

**検査 11 と 12 のスコープは非対称**（意図的）。11 は `docs/original-docs/` 配下**全体**、
12 は **0 埋め 4 桁 ADR だけ**。この結果、非 ADR 一次資料（導入時点で
`601-axis-flip-in-predict-watch.md` の 1 本）と `docs/qa/`（4 本）はどの `sources` からも
参照されていなくても鳴らない。

**この文書自身は `doc_class` を持たない**（クラス定義そのものなので）。したがって ADR 0082 の
写しは `scripts/mdq search --tags D..` の絞り込みからは引けない——運用規約として探すときは
[README.md](README.md) の「昇格・更新の運用」と「機械検査できない」節を入口にする。

**検査 11 は `出典` セルの Markdown リンクしか見ない。** 出典をプレーンテキストや
インラインコードで書けば突合されないので、**最も安い回避策は「出典のリンクを外す」**になった
（塞いだ「`sources` から行を消す」より安い）。一律必須にしなかったのは、既存の出典に
`ADR 0001` のような素のテキスト表記と外部 URL のみの行が実在するため。

### なぜ error か / なぜ例外を文書側に置くか（ADR 0082 の理由と却下案）

- **error にする理由**: #580 が stale を warning → error に昇格させたのは「warning のままだと
  写した量に比例して追従漏れが静かに溜まる」を実測したため。同じ検査系に warning を混ぜると
  その 1 項目だけが同じ経路で腐る。現状の違反が 0〜1 件なので導入コストを払わずに error にできた。
- **却下**: スクリプト内の定数リストで例外を持つ（例外の追加が「Python の変更」になり文書
  レビューの視界から外れる）/ どちらかを warning に留める（上記の理由）/ `sources` を
  append-only にする（穴は塞がるが文書の分割・統合で消せなくなり `sources` が単調増加する）/
  本文の相対リンク全部を `sources` と突合する（用語集や兄弟仕様への相互リンクまで載せることに
  なり、`sources` の意味＝蒸留元が壊れる）/ orphan 判定に `CLAUDE.md` からの参照も数える
  （ADR 0077 で `CLAUDE.md` は `sources` に入らない設計にしたので、「`CLAUDE.md` に書いたから
  写しは不要」という抜け道ができる）。
- **影響**: ADR を先に置いて `sources` への登録を後続 PR に回す運用は塞がる。`--warn-only` は
  逃げ道に数えない（CI では使わない）。既存 10 項目の検査・`scripts/bump-distilled-sha.py` が
  パースする STALE 行の文言は変えていない。
