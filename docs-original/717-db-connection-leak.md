# #717 共有 DB の接続の取り残し — 切り分けの実測ログ（2026-10-04）

pool を閉じずに終わるプロセスの接続が、共有 DB のサーバ側に idle のまま残り続け、`max_connections=100` を使い切る原因の切り分け記録。
方針の確認は `qa/QA-db-connection-leak-717.md`、蒸留先は `knowledge/app-bootstrap.md`（決定ログ #717）。
対象はローカル開発専用の共有 DB（ポートは 127.0.0.1 にだけ公開・外部には出さない）。

## 1. 接続の経路

ホストのアプリ → `limactl`（ホスト 127.0.0.1:5432 を LISTEN。Lima 2.1.1）→ VM 内 127.0.0.1:5432 →
rootlesskit（`--port-driver=builtin`・`--net=slirp4netns`）→ コンテナ `paddock-postgres`（postgres:17-alpine）。
サーバから見た接続元はすべて `10.4.1.1` で、`pg_hba.conf` の `host all all all scram-sha-256` に当たる。

調査の開始時点（開催日 2026-10-04 10:4x）で、サーバ側に idle の client backend が 20 本（最古 12 時間 15 分）あった。
一方、ホスト側の `lsof -nP -iTCP:5432 -sTCP:ESTABLISHED` は 0 本だった（`limactl` を除く）。

## 2. 実験（1 接続ずつ・application_name `t717-*` で識別）

| # | 経路 | クライアントの終わり方 | サーバ側のバックエンド |
|---|---|---|---|
| E1 | ホスト → limactl → rootlesskit | psql を正常終了（Terminate を送る） | 消えた |
| E2 | ホスト → limactl → rootlesskit | psql を kill -9（Terminate なし・ソケットの切断だけ） | idle のまま残った（15 秒後・27 秒後・1 分 32 秒後も残存） |
| E2b | 同上 | python の最小クライアント（SCRAM 認証を自前実装）を kill -9 | idle のまま残った |
| E3 | VM 内 → rootlesskit（limactl を経由しない） | E2b と同じクライアントを kill -9 | 消えた |

- E2 の後、ホスト側に psql のプロセスもソケットも無かった。
- 同じ時点で、VM 内の `ss -tnp state established '( dport = :5432 )'` は全 22 本が `lima-guestagent` で、
  サーバ側の idle の client backend 22 本と一致した。
- 検証で作った `t717-*` の 2 本は、記録の後に `pg_terminate_backend` で切った。

## 3. sqlx 0.9 の close の挙動（ソースで確認）

- `PgConnection::close` は Terminate を送ってから shutdown する。`close_hard` は Terminate を送らない
  （`sqlx-postgres-0.9.0/src/connection/mod.rs`）。
- `PgPool::close` と、idle_timeout / max_lifetime の回収は `close` を使う。`close_hard` を使うのは、ping に失敗したときなど、
  接続が壊れている場合だけ（`sqlx-core-0.9.0/src/pool/inner.rs`・`pool/connection.rs`）。
- `PoolOptions` の既定値は idle_timeout 10 分、max_lifetime 30 分、min_connections 0、test_before_acquire true
  （`sqlx-core-0.9.0/src/pool/options.rs`）。

## 4. 取り残しを生む経路（コード所見）

- 全 app の `setup.rs` は `pool::connect_checked` で pool を作るが、`main` の終わりに閉じていたのは
  `fill-results apply` だけだった。それも `check_db_name(...)?` などで途中 return する経路では閉じない。
- `connect_checked` 自身も、pool を作った後に Err で返す経路（auto_migrate の失敗、`check_migration_status` の失敗、
  Pending、Uninitialized）で閉じずに drop する。
- `src/apps/api-server/tests/openapi_route_parity.rs` は、共有 DB への lazy pool で全ルートを叩き、閉じずに終わる
  （DB の要らないルート解決の検査なので、つながらないアドレスの lazy pool に替えた）。
- launchd の prefetch-odds（5 分毎）は、単発の `paddock-fetch-card` を起動する。2026-10-03 に観測した
  「常駐プロセスだけで約 2 本 / 5 分増えた」は、この経路で説明が付く（推測。predict-watch / odds-collect は
  DB を使う子プロセスを起動していない）。
- `scripts/predict-check/` の DB 参照は psql のサブプロセス（`pgq.py`）で、psql は Terminate を送って終わる。

## 5. テストでの再現（`rdb-gateway/tests/test_pool_close.rs`）

- 閉じない仮実装の `close_after` と、修正前の `connect_checked` では、4 本とも RED になった。
  `connect_checked` の 2 本は、Err で返った後もサーバ側に接続が 1 本残った（`left: 1`）。
- RED の実行で残った接続のせいで、次の実行は sqlx の一時 DB の作り直し（`dropdb`）が
  `55006 ... There are 2 other sessions using the database.` で落ちた。#717 の 2026-10-03 のコメントと同じ症状。
  ユーザーの承認を得て、`_sqlx_test%` の接続だけを切った。
- 実装後は 4 本とも GREEN で、実行後に `_sqlx_test%` の接続は 0 本だった。`connect_checked` の Err 経路の `close` を
  外す変異で 2 本が RED になることも確かめた。この 2 本が RED になるのは Lima のポート転送越しだけで、DB へ直接つながる環境（CI）では、
  閉じずに drop してもソケットの切断がサーバに届くので、修正前でも通る。`verify` の Err 経路は 4 つ（auto_migrate の失敗・整合チェックの失敗・
  Pending・Uninitialized）あり、テストはそのうち代表の 2 つ。4 つとも同じ Err の分岐を通る。
- テスト内の観測用の接続も、閉じずに drop すると同じように残った（初回の GREEN の後に 4 本）。テストでは最後に閉じる。

## 6. その他の確認

- `paddock` ロールは superuser だった（`rolsuper = t`）。2026-10-03 の issue コメントの「superuser ではない」は誤り。
  最小権限のロールへの分離は #759 に切り出した（#717 のスコープ外）。
- コンテナの実マウントは named volume `deployments_paddock-pgdata`。compose（project `deployments`）から導かれる名前と一致した。

## 7. `#[sqlx::test]` 自体が残す接続（実測）

対象 crate のテストをまとめて流したところ、共有 DB が `too many clients already` になった（2026-10-04 15:00）。
VM 内のバックエンドは全 100 本が `paddock` DB の idle で、57 本が 10 分未満だった（ユーザーの承認を得て、1 分以上 idle のものに SIGTERM を送った）。
そこで、空に近い状態からテストバイナリを 1 本ずつ流し、`paddock` DB に残った idle の数を数えた。

| テストバイナリ | テスト数 | 実行後の累計 |
|---|---|---|
| （開始時） | — | 0 |
| `rdb-gateway --test test_pool_close` | 4 | 4 |
| `rdb-gateway --test test_migration_status` | 8 | 12 |
| `api-server --test openapi_route_parity`（この時点では末尾で close する版。後でつながらないアドレスに替えた） | 2 | 12 |
| `api-server --test session` | 9 | 21 |
| `predict --test overview` | 5 | 26 |

- `#[sqlx::test]` は、成功してもテスト 1 本ごとに、`DATABASE_URL` の DB への接続を 1 本残す。sqlx のテストハーネス側の挙動で、アプリのコードでは塞げない。
- `DATABASE_URL` に `?options=-c%20idle_session_timeout%3D20s` を付けて `test_migration_status` を流すと、テストは全部通った。
  実行直後に 8 本増えた分（26 → 34）は、28 秒後に消えた（34 → 26）。セッション単位の設定なので、テストの接続にしか効かない。
- 上の表の `api-server --test session` は、DB が満杯だった初回の実行では 3 本失敗した。掃除した後は 9 本とも通った（失敗は接続の枯渇によるもので、今回の変更とは無関係）。

## 8. 反映と実地検証（PR #760 のマージ後・2026-10-04 19:05〜19:40・開催の後）

反映の手順: primary で `cargo build --release` → `paddock-api` を止める → `scripts/backup-db.sh` で dump（61MB）→
`limactl shell paddock -- nerdctl compose -f deployments/compose.yaml up -d postgres`（nerdctl は「Re-creating container」と出して作り直した。
volume `deployments_paddock-pgdata` はそのまま使われた）→ `paddock-api` を新しいバイナリで起動し直す。
launchd の prefetch-odds は load されていなかった（計測に余計な接続は混ざっていない）。

| 検証 | 手順 | 結果 |
|---|---|---|
| G3 設定の反映 | `SHOW idle_session_timeout` / `SHOW idle_in_transaction_session_timeout` | `30min` / `10min`（ホストの psql からも `30min`）。`race_odds_snapshots` は作り直しの前後とも 7,508,469 行 |
| G4-1 本文の再現手順 | `snapshot_ev_report.py --from 2026-08-09`（35R 分の `paddock-analyze predict` を順に起動・31 秒） | client backend は実行前 2 本 → 直後 2 本 → 5 秒後 2 本（+0 本）。修正前はこの 1 回で 100 本が埋まった |
| G4-2 取り残しの回収 | python の最小クライアントを kill -9（19:08:37 に接続） | 19:38:37 に `FATAL: terminating connection due to idle-session timeout` で切られた（ちょうど 30 分） |
| G4-3 api-server の Ctrl-C | DB を使う API を 4 本叩いて接続を 6 本に増やし、SIGINT | 止めた後は 1 本（計測の問い合わせ自身）。actix の graceful stop の後に `close_after` で閉じても Terminate は届いた |
| G4-4 fill-results の拒否経路 | `paddock-fill-results apply --db-name not_this_db` | 「接続先の DB は paddock です」で拒否された後、接続数は前後とも 2 本 |
