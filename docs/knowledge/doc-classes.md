---
status: Confirmed
kind: knowledge
sources:
  - docs/original-docs/0073-adr-into-original-docs-and-doc-classes.md
distilled_from_sha: "9538782"
updated: "2026-08-09"
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
| D01 | 事業意図・成功条件定義書 | active | 0 |
| D02 | スコープ・対象境界定義書 | active | 1 |
| D03 | ステークホルダー・承認権限・責任分担表 | n/a | 0 |
| D04 | 業務プロセス仕様書 | active | 1 |
| D05 | ユースケース・シナリオカタログ | active | 0 |
| D06 | 業務ルール・判定表仕様書 | active | 1 |
| D07 | 用語集・ドメインモデル定義書 | active | 0 |
| D08 | データモデル・SoR/SoT・データ品質仕様書 | active | 6 |
| D09 | システムコンテキスト・責任境界・再利用方針書 | active | 2 |
| D10 | API / Event / File 連携契約パック | active | 11 |
| D11 | 画面・UX・操作意味仕様書 | active | 7 |
| D12 | 権限・認可・職務分掌設計書 | n/a | 0 |
| D13 | セキュリティ・プライバシー・監査・法規マトリクス | n/a | 0 |
| D14 | 国際化・地域差分仕様書 | n/a | 0 |
| D15 | 非機能・運用・監視・DR 仕様書 | active | 3 |
| D16 | 移行・導入・ロールアウト計画書 | active | 0 |
| D17 | 品質保証・UAT・受入パッケージ | active | 1 |
| D18 | Prompt ガバナンス・入力統制パック | active | 0 |
| D19 | ソフトウェアアーキテクチャ・ADR パック | active | 10 |
| D20 | セキュア設計・実装ガードレール | n/a | 0 |
| D21 | CI/CD・ビルド・リリース・供給網管理仕様書 | active | 0 |
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
警告が恒久的なノイズになり、本当の欠落（D01 / D07 / D21）が埋もれる。

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
| **D01** 事業意図・成功条件 | プロダクトの目標・成功条件・非目標を書いた文書が `docs/` に 1 本も無い。方向性は ADR 73 本を読み解くことでしか復元できない | [#579](https://github.com/taito-station/paddock/issues/579) の PR3 で `product-goals.md` を新設 |
| **D07** 用語集・ドメインモデル | 用語定義が各仕様書に散在している（`win_prob` / `place_prob` / `raw_score` / `blended` / `軸ロック` 等を `probability-estimation.md`・`backtest.md`・`ev-kelly-bet-selection.md` がそれぞれ独自に定義） | 横断的な用語集を 1 本作る |
| **D21** CI/CD・供給網 | `.github/workflows/ci.yml` に 10 ジョブが実在するが、その設計意図（必須チェックの構成・ジョブ分割の理由・shellcheck の範囲）を書いた文書が無い | ADR は個別に存在（0026 等）。横断的な 1 本が要る |
| **D05** ユースケース | `README.md` の「何ができるか」が最も近いが、UC カタログの形では無い | 優先度低 |
| **D16** 移行・導入 | ADR 0070（DB マイグレーション運用）が近いが移行計画書ではない | 優先度低 |
| **D18** Prompt ガバナンス | 実質は [`docs/knowledge/README.md`](README.md)（3 層モデル・SoT の優先順位）と `CLAUDE.md` が担っている | 名前だけ空。当面このままでよい |

## 体系側の既知の穴

**D23（買い方・資金配分ルール）の一次定義がリポジトリの `docs/` 配下に無い。** 現行ルールの本体は
プロジェクトルートの `CLAUDE.md`「買い方ルール」節にあり、`docs/` 側の D23 文書
（`betting-rule-history.md` / `live-ev-buy-view.md` / `ev-kelly-bet-selection.md`）は
**根拠・棄却記録・画面契約**に留まる。`CLAUDE.md` は毎セッション読まれる運用指示なので現状で機能して
いるが、「クラスの主文書がクラス体系の外にある」状態ではある。D01 を作るとき（#579 の PR3）に
併せて整理を検討する。

## 割当の一覧

各文書の `doc_class` は **frontmatter が正**で、この表は読みやすさのための索引。

> checker が突き合わせるのは上の一覧の**クラス別集計数だけ**で、この索引表は読んでいない。
> 主クラスの順序入替や 2 文書間のクラス交換は集計が変わらないので検出されない。
> `doc_class` を変えたらこの表も手で直すこと。

| 文書 | doc_class |
|---|---|
| knowledge/analyze-search-and-state.md | [D11, D10] |
| knowledge/app-bootstrap.md | [D19, D15] |
| knowledge/live-freshness-calibration.md | [D11, D10] |
| knowledge/monitor-loop-sleep-resilience.md | [D15, D19] |
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
