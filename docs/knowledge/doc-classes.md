---
status: Confirmed
kind: knowledge
sources:
  - docs/qa/QA-sources-coverage-checks-596.md
distilled_from_sha: "daf3beb"
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
| **D18** Prompt ガバナンス | 実質は [`docs/knowledge/README.md`](README.md)（2 層モデル・SoT の優先順位）と `CLAUDE.md` が担っている | 名前だけ空。当面このままでよい |

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
**移すかどうかを決めるのは決定ログの仕事**なので、この段落は決定記録ではなく現状の説明として読むこと。

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

## `sources` の網羅性検査（ADR 0083）

stale 検査（`sources` に挙げた一次資料への追従）の**前提**を守る検査。
「`sources` の中身が正しいか」を見る側で、これが無いと stale 検査は素通りさせられる。

### REQ 表の出典も `sources` に載せる

stale 検査が見るのは `sources` に**挙がっている**行だけなので、**`sources` から行を消せば
stale も消える**。その穴を塞ぐため、**REQ 表の `出典` 列が名指しした `docs/docs-original/`
配下のファイルは、その文書の `sources` にも載っている**ことを checker が error で保証する
（ADR 0083 決定 2）。`出典` 列は「その要件の根拠」と定義された唯一の機械可読な場所なので、
ここが指した一次資料だけは必ず watch 対象に入る。

- **基準パスはリポジトリルート相対に正規化してから比較する。** `出典` 列は文書からの相対
  （`../docs-original/571-....md`）、`sources` はルート相対で、揃えないと必ず食い違う。
- **対象は `docs/docs-original/` 配下全体**。
- **対象外**: 外部 URL（GitHub issue 等。一次資料ファイルではないので `sources` に載せられない）/
  兄弟の knowledge・specifications へのリンク（蒸留元ではなく相互リンク）/ リンク切れ
  （本文リンク検査が別に報告する担当。ここで拾うと 1 本の切れリンクに 2 件の error が出るうえ、
  `sources` は実在ファイルしか受け付けないので「`sources` に足せ」が誤った助言になる）。

### この検査が保証しないこと

**塞げたのは「REQ が根拠として名指しした出典」だけ。** REQ 表の外で本文が根拠にしている
一次資料を `sources` から落とす操作は検出されない（stale もろとも静かに消える）。
**出典は減らさない**という規律が引き続き要る。

**`出典` セルの Markdown リンクしか見ない。** 出典をプレーンテキストやインラインコードで
書けば突合されないので、**最も安い回避策は「出典のリンクを外す」**になった（塞いだ
「`sources` から行を消す」より安い）。一律必須にしなかったのは、既存の出典に
`ADR 0001` のような素のテキスト表記と外部 URL のみの行が実在するため。

**`docs/qa/` は対象外。** どの `sources` からも参照されていない qa ファイルがあっても鳴らない。

**この文書自身は `doc_class` を持たない**（クラス定義そのものなので）。したがって
`scripts/mdq search --tags D..` の絞り込みからは引けない——運用規約として探すときは
[README.md](README.md) の「昇格・更新の運用」と「機械検査できない」節を入口にする。

### なぜ error か（ADR 0083 の理由と却下案）

- **error にする理由**: #580 が stale を warning → error に昇格させたのは「warning のままだと
  写した量に比例して追従漏れが静かに溜まる」を実測したため。同じ検査系に warning を混ぜると
  その 1 項目だけが同じ経路で腐る。導入時点の違反が 0 件なので導入コストを払わずに error にできた。
- **却下**: warning に留める（上記の理由）/ `sources` を append-only にする（穴は塞がるが
  文書の分割・統合で消せなくなり `sources` が単調増加する）/ 本文の相対リンク全部を `sources` と
  突合する（用語集や兄弟仕様への相互リンクまで載せることになり、`sources` の意味＝蒸留元が壊れる）。
- **影響**: ADR を先に置いて `sources` への登録を後続 PR に回す運用は塞がる。`--warn-only` は
  逃げ道に数えない（CI では使わない）。既存 10 項目の検査・`scripts/bump-distilled-sha.py` が
  パースする STALE 行の文言は変えていない。

---

## 決定ログ

<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->

### ADR 0083: `sources` の網羅性を機械検査にする（orphan ADR / REQ 出典の突合） (2026-08-14) — 承認済み

#### ステータス

承認済み。ADR 0073 の後続（[#579](https://github.com/taito-station/paddock/issues/579) の段階的タスク・
[#596](https://github.com/taito-station/paddock/issues/596) / [#597](https://github.com/taito-station/paddock/issues/597)）。

#### コンテキスト

ADR 0073 は「ADR の内容は knowledge へ全部写す。重複を許す代わりに同期切れは機械で検出する」を選び、
その担保として `scripts/check-doc-classes.py` の **stale 検査**（`sources` に挙げた一次資料が
`distilled_from_sha` より後に更新されていたら error）を置いた。

**この担保には網羅性の穴がある。** stale が見るのは「`sources` に**挙がっている**行」だけで、
`sources` の**中身が正しいか**は誰も見ていない。

- **`sources` から行を消せば stale も消える。** 追従が面倒な出典を削るだけで検査を黙らせられる。
- **ADR を足したときに `sources` へ載せ忘れても、何も起きない。** その ADR が後で更新されても
  下流は「追従すべき文書」として認識されないまま静かに古くなる。
- **REQ 表の `出典` 列が名指しした ADR も同じ。** 本文では根拠として挙げているのに `sources` に
  無ければ、その根拠が変わっても要件側は気づかない。

実測（main `5ae6466` 時点）では不変条件はまだ真だった。

| 検査対象 | 母数 | 違反 |
|---|---|---|
| ADR の被参照（どこかの `sources` にあるか） | ADR 80 本 | **1 本**（`0074-no-issue-body-transcription.md`） |
| REQ 表 `出典` 列 ⊆ 同文書の `sources` | REQ 行 26 / 出典のリポ内リンク 37（ユニーク 29） | **0 件** |

**真であるうちに検査を入れないと、次に ADR を足した誰かが静かに壊す。** これは ADR 0073 / 0074 が
繰り返し排除してきた「人手の規律に委ねる」構図そのもので、`app-bootstrap.md` の `NoopParser` 事故
（status が Confirmed のまま存在しない実装を推奨し続けた）と同型の経路。

#### 決定

`scripts/check-doc-classes.py` に検査を 2 つ足し、例外は文書側で宣言する。

1. **orphan ADR 検査 [error]**（#596）。`docs/docs-original/` の **0 埋め 4 桁の ADR**（判定の
   述語は決定 4）を列挙し、
   全 knowledge / specifications の `sources` の和集合に含まれないものを error にする。
   issue 由来の一次資料（`382-...` のように 0 埋めしない番号）は蒸留先を持つとは限らないので対象外。

2. **REQ 出典 ⊆ sources 検査 [error]**（#597）。REQ 表の `出典` セルが名指しした
   **`docs/docs-original/` 配下のファイル**が、その文書の frontmatter の `sources` に無ければ error。
   GitHub issue の絶対 URL など**リポジトリ外の参照は対象外**（一次資料ファイルではないので
   `sources` に載せられない）。knowledge / specifications 同士の相互参照も対象外
   （蒸留元ではなく相互リンクなので、`sources` に載せる筋合いが無い）。
   基準パスは**リポジトリルート相対に正規化**して比較する（`出典` 列は文書からの相対、
   `sources` はルート相対で、片方に寄せないと必ず食い違う）。

3. **例外は `docs/knowledge/doc-classes.md` のマーカー付き宣言表で持つ。**
   `<!-- adr-orphan-exceptions:begin -->` … `:end` の 2 列表（`| ADR | 例外の理由 |`）に、
   **理由を必須**で書く。パスは `sources` と同一形式（リポジトリルート相対）にする。
   現時点の登録は **ADR 0074 の 1 本のみ**——文書運用の規約そのものを定めた ADR で、
   蒸留先の knowledge を持たない。

   **正当な例外カテゴリは 2 つ**: (a) 規約そのものを定めた ADR で写す先が無いもの、
   (b) **supersede されて下流が後継 ADR だけを参照するようになった ADR**。(b) は
   「決定を変えるときは新しい ADR で supersede する」（CLAUDE.md）と直交する必然で、
   下流が旧 ADR を `sources` から落とすのは正しい操作。落とした瞬間に orphan になるので、
   **supersede する PR で例外表に「ADR NNNN に supersede された」と書く**。

4. **ADR の判定は `scripts/check-adr-numbers.sh` と同一の述語（`^0[0-9]{3}`）にする。**
   先頭 0 を落とすと、issue 番号が 4 桁に届いた時点で issue 由来の一次資料
   （`1024-foo.md`）が ADR と誤判定され、「例外表に登録しろ」という誤った助言つきで
   CI が落ちる。judge を 2 本に割らない——本 ADR が塞いでいる second source と同型。

5. **例外表そのものも検査する。** 実在しない ADR／実在するが ADR ではないファイル／実際は
   `sources` から参照されている（＝不要になった例外）／行の重複／理由の空欄（決定 3 の裏返し）／
   見出し行の列名違い／行の書式崩れ、はいずれも error。N/A 宣言表と一覧の相互突合
   （ADR 0073 で入れた同型の検査）と同じ考え方で、**腐った例外を残さない**。

6. **`sources` と例外表のパスは正規形（`docs/...`）だけを許す。** `./docs/...` のような
   非正規形は実在検査を通るのに **stale 判定の突合から静かに外れる**（`path_status` が
   `git show --name-status` の出力と終点一致で突き合わせるため、`./` 付きは永久に一致せず
   「履歴を辿れず」の warning に退化する）。突合用に正規化した集合を別に持つ手もあるが、
   それは「どちらの形式か」を持つ場所を増やす——例外表のパス形式で却下したのと同じ理由で、
   **形式を 1 つに強制する**方を採る。導入時点で `sources` の全行・例外表 1 行すべて正規形
   （大文字小文字違いも同じ理由で error にする。macOS では実在検査を通り Linux の CI だけが
   落ちるうえ、手元では stale が warning に退化する）。

7. **ADR が 0 件なら error にする。** 判定条件やディレクトリの取り違えで検査 12 が丸ごと
   無言で無効化される fail-open を塞ぐ。これは「違反がある」ではなく**「検査が成立していない」**
   側の失敗なので、マーカー欠落と同じく **`--warn-only` でも抑止しない**。

#### この検査が保証しないこと（意図的な限界）

**機械化できたのは「`sources` への登録」までで、「knowledge へ写したか」ではない。**
ADR を任意の文書の `sources` に 1 行足せば検査 12 は通る（実際
`docs/knowledge/product-goals.md` は 34 本の ADR を索引目的で `sources` に並べている）。
**写しの中身——決定・理由・却下案・影響を実際に書いたか——は人手の規律に残る。**
ADR 0073 決定 2 の担保としては部分的で、ここを誇張して書くと
「機械が見ているから大丈夫」という誤った安心を生む。

**参照元が複数ある ADR には、元の穴が開いたまま残る。** 導入時点（`ffdace0`）で
**ADR 83 本のうち 48 本（58%）が 2 文書以上から参照されている**
（被参照数の分布 `{0:1, 1:34, 2:21, 3:15, 4:5, 5:2, 6:3, 7:1, 8:1}`）。
これらは 1 つの文書の `sources` から落としても検査 12 は鳴らず（他がまだ参照している）、その文書の
REQ `出典` に載っていなければ検査 11 も鳴らない。**塞げたのは「最後の 1 本を落とす」ケースと
「REQ が根拠として名指しした出典」だけ**で、中間——本文が根拠にしているが REQ 表の外にある参照——は
人手の規律に残る。

**検査 11 と 12 でスコープが非対称**なのも意図的。11 は `docs/docs-original/` 配下**全体**
（issue 由来の一次資料も蒸留元なので）、12 は **0 埋め 4 桁 ADR だけ**（issue 由来の一次資料は
調査所見の置き場で、蒸留先を持つとは限らない）。この結果、導入時点で非 ADR 一次資料 1 本
（`601-axis-flip-in-predict-watch.md`）と `docs/qa/` の 2 本（`QA-axis-lock-601.md` /
`QA-roi-gate-calibration-571.md`。各層の `README.md` は数えない）がどの `sources` からも
参照されていないが、これらは検査対象外なので鳴らない。

同様に、検査 11 は **`出典` セルの Markdown リンクしか見ない**。出典をプレーンテキストや
インラインコードで書けば突合されないので、**最も安い回避策は「出典のリンクを外す」**に
なった（塞いだ「`sources` から行を消す」より安い）。ここを error にしなかったのは、
既存の出典に `ADR 0001` のような素のテキスト表記と外部 URL のみの行が実在し、
一律必須にすると本題と無関係な修正を大量に強いるため。**残る穴として
`docs/knowledge/README.md` の「機械検査できない」リストに明記する**。

#### 理由

- **stale 検査は `sources` が正しいことを前提にしている。** 前提の側を無検査にしたまま
  結論の側だけ機械化しても、抜け道が残っているぶん fail-open になる。片方だけ締めても意味が薄い。
- **error でなければ守れない。** #580 で stale を warning → error に昇格させたのは、
  「warning のままだと写した量に比例して追従漏れが静かに溜まる」を実測したため。同じ検査系に
  warning を混ぜると、その 1 項目だけが同じ経路で腐る。**現状の違反が 0〜1 件で、しかもその 1 件が
  既知の例外**なので、導入コストを払わずに error にできる。
- **例外はスクリプトでなく文書に置く。** スクリプト内の定数リストにすると、例外を増やす行為が
  「Python の変更」になり、文書レビューの視界から外れる。宣言表なら**例外を増やすたびに
  理由が文書差分として残り、レビューに乗る**。既存の `extract_block()` をそのまま再利用できるので
  機構の追加もゼロ。
- **例外のパスを `sources` と同形式にする。** 割当索引は `docs/` を剥がした形式だが、
  この表の比較相手は `sources` なので、正規化を 1 段挟むほど「どちらの形式か」の事故が増える。
  比較相手に合わせるのが最も読み違えにくい。
- **REQ の `出典` を選んだのは、そこが「根拠を名指しした」唯一の機械可読な場所だから。**
  本文の相対リンク全部を `sources` に強制すると、単なる相互リンクまで watch 対象になって
  `sources` がノイズで膨らむ。REQ 表の `出典` 列は定義上「その要件の根拠」なので、
  watch 対象であるべきという主張が成り立つ。

#### 却下した代替案

- **例外をスクリプト内の定数リストで持つ**（`ORPHAN_EXCEPTIONS = {...}`）。実装は最小だが、
  例外の追加が文書レビューに乗らない。「規律に委ねない」という ADR 0073 / 0074 の一貫した
  方針に反する——例外こそレビューされるべきもの。
- **どちらか／両方を warning に留める。** 「ADR を先に置いて写しを後続 PR に回す」運用を
  塞がずに済むが、#580 が実証したとおり warning は無視され、機械検査の実効性が落ちる。
  ADR 先置きが本当に必要なら、**例外表に理由付きで登録する**のが正規の逃げ道で、
  そのほうが「なぜ写しが無いのか」が残る。
- **`sources` から行を消せない仕組み（append-only）にする。** 穴の原因そのものを潰せるが、
  出典が本当に不要になったとき（文書の分割・統合）に消せず、`sources` が単調増加する。
  網羅性は「消せない」ではなく「消したら本文と矛盾する」で守るほうが素直。
- **REQ の `出典` だけでなく本文の相対リンク全部を `sources` と突合する。** 網羅性は最大になるが、
  用語集や兄弟仕様への相互リンクまで `sources` に載せることになり、stale の発火が本題と
  無関係な理由で増える。`sources` の意味（蒸留元）が壊れる。
- **orphan の判定に `CLAUDE.md` からの参照も数える。** ADR 0077 で `CLAUDE.md` は `sources` に
  入らない設計にしたので、ここで参照元として数えると「`CLAUDE.md` に書いたから写しは不要」という
  抜け道ができる。ADR 0073 決定 2 が要求しているのは knowledge / specifications への写し。

#### 影響

- **変更**: `scripts/check-doc-classes.py` に検査 11（REQ 出典 ⊆ sources）と 12（orphan ADR）を追加。
  docstring 冒頭の検査項目リストも更新する。
- **変更**: `docs/knowledge/doc-classes.md` に `adr-orphan-exceptions` マーカーブロックと
  本 ADR の写しを追加する。`scripts/test-check-doc-classes.py` の `REGISTRY_TEMPLATE` にも
  同じマーカーが要る（`extract_block()` はマーカー欠落で `sys.exit` する fail-closed のため）。
- **運用の変更**: **ADR を新設したら、同じ PR でどこかの knowledge / specifications の `sources` に
  載せる。** 載せられない ADR（規約そのものを定めた ADR / supersede 済み）は例外表に理由付きで
  登録する。**塞がるのは「`sources` への登録を後続 PR に回すこと」までで、本文の写しを後回しに
  する運用は依然として機械では止まらない**（上記「この検査が保証しないこと」）。
- **既存検査の厳格化**: 決定 6 は検査 4（`sources` の実在）に「非正規形・大文字小文字違いを
  error にする」分岐を足す＝**既存項目の挙動変更**（導入時点で違反 0）。また
  「サブディレクトリの `.md`」を warning から **error に昇格**させる——文書を 1 階層下げるだけで
  frontmatter 系・stale・REQ の一意台帳・`sources` の被参照が丸ごと外れ、警告 1 行のまま
  exit 0 になる（実測）。orphan 検査が入った今は他の ADR の誤判定にも波及するので、
  #580 が stale を昇格させたのと同じ理由で error にする（導入時点で該当 0 件）。
- **不変**: 上記以外の既存検査と `scripts/bump-distilled-sha.py` がパースする STALE 行の文言は
  変えない。`--warn-only` のセマンティクス（違反は警告扱いにして 0 で終了）も変えないが、
  **fail-closed（抑止できない）側に 2 つ加わる**——`adr-orphan-exceptions` マーカーの欠落
  （既存のマーカー欠落と同じ扱い）と、ADR 0 件（決定 7）。
- **並走 PR の競合**: ADR PR は必ず hub 文書の frontmatter（`sources` と `distilled_from_sha` は
  隣接する）を触るため、**同時期の ADR PR 同士はほぼ必ずコンフリクトする**。しかも競合を
  解決してマージしても、後発の `distilled_from_sha` が先発の ADR を含まないので **マージ後の
  main で STALE が出る**。branch protection の "Require branches to be up to date" を有効にするか、
  マージ後に `scripts/bump-distilled-sha.py --all-stale` を回す。
- **必ず 2 コミットになる**: 新設 ADR を `sources` に載せた時点で、その ADR の最終内容変更は
  現 `distilled_from_sha` より後になるので **必ず STALE が発火する**（自分の sha を同じコミットに
  書けないため）。本文コミットの後に `scripts/bump-distilled-sha.py --all-stale` で追従コミットを
  積む。本 ADR 自身もそうなっている。
- **副次的な効果**: 本 ADR 自身が orphan 検査の最初の対象になる（`doc-classes.md` の `sources` に
  載せることで満たす）。検査が自分自身に効いていることの実演になっている。
- 関連: ADR 0073（一次資料層への ADR 統合・stale 検査）/ ADR 0074（転記しない）/ ADR 0077
  （`CLAUDE.md` を `sources` に入れない）/ #579（親）/ #580（stale の error 昇格）/ #594 / #604。

#### 再現方法

```sh
# 検査が通ること（false positive が無いこと）
scripts/check-doc-classes.py            # → exit 0

# orphan 検査が効くこと: 例外表から 0074 の行を一時的に外す
# → 「どの knowledge / specifications の sources からも参照されていない」で exit 1

# REQ 出典検査が効くこと: ev-kelly-bet-selection.md の sources から ADR を 1 本外す
# → 「出典 ... が frontmatter の sources に無い」で exit 1（従来は stale ごと消えて素通りだった）

# 回帰テスト
python3 scripts/test-check-doc-classes.py
```
