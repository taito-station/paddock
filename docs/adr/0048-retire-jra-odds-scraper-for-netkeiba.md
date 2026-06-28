# ADR 0048: ライブオッズ取得を JRA から netkeiba へ統一し odds-scraper を撤去 (Issue #287)

## ステータス

承認済み（ADR 0001 の「ライブ遷移層 `UreqOddsScraper`」を supersede。ADR 0005 / 0010 / 0019 が
前提とする live odds 取得経路を netkeiba へ置換）

## コンテキスト

`paddock-predict-watch`（発走直前の EV/ROI 監視, #257）のライブオッズ再取得が**全レースで失敗**し、
`response was not valid EUC-JP` 警告とともに `オッズ未取得（未公開/失敗）` でスキップされ、ROI 判定が
一切できない不具合が報告された（#287、2026-06-28 函館・小倉・福島の対象 7 レース全滅）。

切り分けの結果、根本原因は **live odds 取得に 2 系統が併存し、predict-watch 経路だけが
実質機能していない JRA 経路に配線されていた**ことだった。

- JRA 経路（ADR 0001 の `odds-scraper` crate / `UreqOddsScraper`）: `accessO.html` に **`cname`
  セッショントークンを POST** して辿る best-effort 実装で、ADR 0001 時点から**ライブ実地未検証**。
  実際には内部 `RaceId` 文字列を cname として POST するためレスポンスがナンセンスになり、
  それを **EUC-JP 固定デコード**（`scraper_util::decode_euc_jp`）して `response was not valid
  EUC-JP` 警告 → 空オッズに畳まれていた。開催日以外はページ自体が存在しない制約もある。
- netkeiba 経路（`UreqNetkeibaScraper` / `NetkeibaScraper`）: オッズ API
  （`api_get_jra_odds.html?type=1/4/5/6/7/8`）を **UTF-8 JSON** で取得する。#102 / #187 で単複に加え
  組合せ券種（馬連・ワイド・馬単・三連複・三連単）も netkeiba から取得するようになり、fetch-card の
  オッズ保存はこの経路で正常動作している（#287 でも fetch-card は函館5R を 212 件正常保存）。

`OddsInteractor<O: OddsScraper, R>` はジェネリックで、predict / predict-watch / api-server の 3 アプリが
いずれも `O = UreqOddsScraper`（JRA）を注入していた。predict / api-server は read-through キャッシュ
優先（保存済みがあれば再スクレイプしない, ADR 0010）のため fetch-card 保存分でマスクされ顕在化して
いなかったが、キャッシュ無時に live scrape へ落ちると同じ EUC-JP 全滅を起こす**潜在バグ**だった。

## 決定

1. **`UreqNetkeibaScraper` に `OddsScraper` を実装する。** 内部 `RaceId` を
   `netkeiba_race_id_from_paddock` で 12 桁へ変換し、`fetch_win_place_odds` / `fetch_exotic_odds`
   （UTF-8 JSON 経路）を fetch-card 同様のベストエフォートで呼び、純関数 `assemble_netkeiba` で
   `RaceOdds` に組み立てる。fetch-card と live scrape の取得経路が単一の netkeiba 実装に揃う。
2. **live odds 取得を全アプリで netkeiba へ統一する。** predict / predict-watch / api-server の
   `OddsInteractor` の型引数を `UreqOddsScraper` → `UreqNetkeibaScraper` に差し替える。
3. **JRA `odds-scraper` crate を撤去する。** `UreqOddsScraper` / `OddsPages` / `assemble` /
   JRA odds HTML パーサと fixture を削除し、workspace members・`workspace.dependencies`・各アプリの
   依存からも除去する。use-case の port トレイト `OddsScraper` と `OddsInteractor` のジェネリクスは
   据え置く（実装差し替えを許す抽象として引き続き有効）。

## 理由

- **根本原因を一度で潰す**: predict-watch だけ差し替えても、predict / api-server に同一の壊れた scraper が
  残る。一時的修正を避け、live odds 取得を実証済みの netkeiba 一本に統一する。
- **取得経路の単一化**: fetch-card（保存）と live scrape（監視・read-through）が同じ netkeiba 実装・
  同じ UTF-8 デコード・同じパーサを共有し、エンコーディング不整合が再発しない。
- **開催日制約の解消**: netkeiba 経路は race_id ベースの GET で確定後も最終オッズを返すため、ADR 0001 の
  「開催日以外はページ自体が存在しない」「ライブ実地未検証」という制約が無くなる。
- **dead code を残さない**: ADR 0001 の JRA 経路は実地で機能しないことが #287 で確定したため、crate ごと
  撤去して将来の誤配線を防ぐ。

## 影響

- predict-watch の発走直前 ROI 監視が機能する（#287 解消）。predict / api-server の live scrape
  フォールバックも netkeiba 経路になり潜在バグが解消する。
- `assemble_netkeiba` は f64 → `OddsValue`/`PlaceOdds`（finite かつ `>= 1.0`）変換に失敗する行を
  その行だけ skip する（取りこぼし耐性）。組合せ券種は DTO 段階でドメイン型キーを持つためキー変換は不要。
- ADR 0001 / 0005 / 0010 / 0019 が文中で参照する `odds-scraper` / `UreqOddsScraper` は本 ADR 以降
  存在しない（live odds 取得は `UreqNetkeibaScraper` の `OddsScraper` 実装が担う）。これらの ADR は
  歴史的記録として原文を残し、本 ADR で supersede する。
- 検証: 純関数 `assemble_netkeiba` の単体テスト（全券種組み立て・不正行 skip・空入力）に加え、
  #287 で全滅していた函館5R（`2026-1-hakodate-6-5R`）を新 `scrape()` 経路で live 取得し、
  EUC-JP 警告なしに全券種取得できることを確認した（win6/place6/quinella15/wide15/exacta30/trio20/
  trifecta120）。
