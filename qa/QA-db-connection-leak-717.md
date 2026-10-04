# QA: 共有 DB の接続の取り残し（#717・2026-10-04）

#717 の切り分けの後、方針を決めたときの確定事項の記録。一次資料: `docs-original/717-db-connection-leak.md`。
蒸留先: `knowledge/app-bootstrap.md`（共通ヘルパ・決定ログ #717）。

## Q1: どう直すか

- 選択肢: **アプリで pool を close し、サーバに `idle_session_timeout` を入れる**（推奨）/ それに加えて Lima のポート転送方式も切り替える /
  `idle_session_timeout` だけ入れる。
- 回答: **アプリで close ＋ `idle_session_timeout`**。Lima の転送方式の切り替えは VM の再起動が要るので、今回は対象外。
- 理由: アプリ側の close は、短時間に何十回も起動する形（analyze の多重起動）を防ぐ。サーバ側の回収は、panic・kill・テストの失敗など、
  アプリでは塞げない経路を拾う。どちらか片方だと、どちらかの経路が残る。

## Q2: 計画の承認（敵対的レビュー反映後）

- 回答: **承認**。レビューで見つかった穴（`connect_checked` 自身の Err 経路・共有 DB に接続するテスト）も対象に含めた。
- desktop（dioxus の GUI）は対象外。pool の寿命がウィンドウに付き、閉じずに残った分はサーバ側の回収に任せる。

## Q3: 共有 DB の接続の掃除（作業中の確認）

- テストの RED 確認で自分が残した、`_sqlx_test%` の一時 DB の接続だけを `pg_terminate_backend` で切ることを承認した。
- テストをまとめて流して `paddock` DB が上限 100 本に達したとき（大半は `#[sqlx::test]` の取り残し）と、計測の後に、
  VM 内で 1 分以上 idle のバックエンドに SIGTERM を送ることを、その都度承認した（paddock-api の接続は sqlx が張り直す）。

## Q4: セルフレビューで出た判断事項（2026-10-04）

- `idle_in_transaction_session_timeout` も入れるか: **10min で入れる**。`idle_session_timeout` はトランザクション中の idle に効かず、
  そこで切れた接続は行ロックを握ったまま残る。rdb-gateway のトランザクションは関数の中で完結するので、正当な処理には当たらない。
- `paddock` ロールが superuser であること: **別 issue（#759）に切り出す**。権限の状態は #717 以前からのもので、この PR では変えない。

## Q5: サーバ設定の反映時期

- `idle_session_timeout` の反映（コンテナの作り直し）と実地検証は、開催の後に行う（#717 の「開催時間帯を避ける」制約）。
