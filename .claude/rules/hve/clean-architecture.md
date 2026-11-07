---
paths:
  - "**/*.rs"
  - "**/*.go"
  - "**/*.py"
  - "**/*.rb"
  - "**/*.php"
  - "**/*.java"
  - "**/*.kt"
  - "**/*.cs"
  - "**/*.ts"
  - "**/*.tsx"
  - "**/*.js"
  - "**/*.jsx"
  - "**/*.vue"
  - "**/*.svelte"
  - "**/*.swift"
  - "**/*.dart"
---

# クリーンアーキテクチャ共通規約

言語を問わず、新しく書くコードのレイヤー構成と依存の向きを揃える規約。手本は [ttm-bros/rust-backend-template](https://github.com/ttm-bros/rust-backend-template)（`develop` ブランチ。ttm-bros の関係者だけが開ける）。

## 原則（言語・バックエンド・クライアント共通）

- 内側は外側に依存しない: 外側（入口・外部アダプター・起動）→ ユースケース → ドメインモデルの向きだけを許す（外側同士は、下のバックエンドとクライアントの表の範囲で許す）。内側は外側の型・ライブラリを知らない
- ドメインモデルは外部依存を持たない（フレームワーク・DB・HTTP クライアントを import しない）
- 外部依存（外部 API・DB・ファイル・時計など）は、ユースケース側に置いた抽象（gateway）越しに使う。実装は外側のレイヤーが持つ
- 依存の組み立て（実装を生成して抽象に差し込む）は外側で行う。ドメインモデルとユースケースは実装を参照しない
- レイヤーごとに独自のエラー型を持ち、外側へ渡すときに変換する。内側のエラーに外側の概念（HTTP ステータスなど）を持ち込まない

## バックエンドのレイヤー構成と命名（バックエンドに限る）

各レイヤーを独立したビルド単位（Rust は crate、他の言語はパッケージ・モジュール・ディレクトリ）にし、できる言語では依存をビルド単位で強制する。

| レイヤー | 置き場所（手本） | 中身 | 依存してよい先 |
|---|---|---|---|
| model | `domain/model/` | エンティティ・値オブジェクト（例: `time/date_time.rs`） | なし |
| use-case | `domain/use-case/` | `interactor/`（ユースケース。1 ユースケース 1 ファイル）、`gateway/`（外部依存の抽象。1 外部依存 1 ファイル）、`errors/` | model |
| controller | `controller/<入口>/`（例: `controller/rest/`） | `handlers/<version>/<resource>/`（`handler` と、入出力の `schema`。ヘルスチェックは `handlers/health/<種類>/`）、`builder`、`errors/` | model・use-case・gateway の実装 |
| gateway | `gateway/<外部サービス名>/` | use-case の gateway の実装。`handlers/<操作>/`（外部の形式からモデルへの変換を含む）、`builder`、`errors/` | model・use-case |
| app | `app/<実行形態>/`（例: `app/rest-server/`） | 起動とルーティングの登録 | controller |

- 命名
  - ユースケースは `Interactor`。ユースケースごとに `interactor/` に 1 ファイルを足し、`Interactor` のメソッドとして実装する（手本: `interactor/get_now_time.rs` の `impl Interactor { async fn get_now_time() }`）
  - 外部依存の抽象は `gateway`（例: `trait WorldTimeApi`）。抽象をまとめる入れ物を `Gateway` とし、`Interactor::new(gateway)` で渡す
  - gateway の実装は `<抽象名>Impl`（例: `WorldTimeApiImpl`）。生成は `<名前>Builder`（`with_<設定>()` を連ねて `build()`）
  - 入口の入出力の型は `schema` に置き、モデルから変換して作る（`From<Model>` など）
- 依存の組み立て: 手本は controller の `InteractorBuilder` が gateway の実装を生成して `Interactor` を作り、app はそれを呼ぶだけ。app で組み立ててもよく、そのときは app が use-case と gateway の実装にも依存してよい
- エラーの変換: gateway のエラーは use-case のエラーへ（`From`）、use-case のエラーは入口の応答へ変換する。応答には内部エラーの詳細を含めず、詳細はログに残す（秘密情報・認証情報・個人情報は出さない）。手本の handler がエラーの文字列を応答に含める点には倣わない

## 未定義の領域

次は手本にも無く、この規約では決めない。各リポジトリで決めて `.claude/rules/hve-local/clean-architecture.md` に書く。

- DB の gateway の構成（リポジトリの単位、トランザクションの境界）
- use-case のエラーの種類と、HTTP ステータスなど入口の応答への対応表
- 認証・認可の置き場所
- ユースケースの入出力専用の型（DTO）を設けるか
- クライアントの状態管理・ルーティング・サーバの状態のキャッシュに使うライブラリ（置くレイヤーはクライアントの節のとおり）
- クライアントが認証情報を保存する端末の領域とライブラリ、Web で端末に保存するかと方式（保持と破棄の責務、保存の原則はクライアントの節のとおり）

Rust と PHP は、言語別規約（下記）がこのうちバックエンドの項目（トランザクションの境界を除く）を定めている。

## 言語別規約との関係

hve-playbook は Rust と PHP に、fridgers-backend 型の言語別規約（hve-playbook の `rules/global/rust/architecture.md`・`rules/global/php/architecture.md`）を user-level の rule として配っている（入っている環境でだけ、該当ファイルを読んだときに載る）。どちらに従うかは次で決める。

- 新しく作るリポジトリ、または既存コードがこの規約の構成なら、この規約に従う。言語別規約は「未定義の領域」の参考にだけ使う
- 既存コードがこの規約の構成でなければ（fridgers 型を含む）、既存コードの構成と命名、原則の適用の度合いに合わせる。fridgers 型で言語別規約が載っていれば、言語別規約に従う。この規約への移行は各リポジトリの Issue で決める

| この規約 | Rust 言語別規約 | PHP 言語別規約 |
|---|---|---|
| `domain/model` | `src/domain` | `app/Domain` |
| `domain/use-case` の `interactor` | `src/use-case` の `Interactor<R>` | `app/UseCase` の `{Action}{Entity}Interactor` |
| `domain/use-case` の `gateway` | `Repository` trait | `{Entity}RepositoryInterface` |
| `controller/rest` | `src/interface/rest-controller` | `app/Interface/Http` |
| `gateway/<service>` | `src/interface/rdb-gateway` | `app/Interface/Persistence/Eloquent` |
| `app/<実行形態>` | `src/apps/<bin>` | Laravel のエントリポイント・`AppServiceProvider` |

## クライアント（フロントエンド・モバイル）

ブラウザ・端末で動くコードに当てる（サーバ側のコードはバックエンドの節に従う）。

- 適用範囲: Web は React の SPA、モバイルは Flutter。それ以外のクライアント（Vue・Svelte・SwiftUI、ファイルの配置でルーティングを決めるフレームワーク（Next.js など））は原則の節だけを適用し、構成は `.claude/rules/hve-local/clean-architecture.md` に書く
- use-case の層は必須。画面の層（Web の presentation・Flutter の ui）は model と use-case（と下の補助・ルートの定数）だけに依存し、外部依存の抽象（gateway・repository）も実装も直接使わない
- ドメインの概念を持たない補助（Web は `src/utils/`、Flutter は `lib/utils/`）は層に属さず、どの層からも使ってよい。補助は層のコードを import しない
- エラーの変換: データの層（gateway・data）のエラーは use-case のエラーへ、use-case のエラーは画面の層で利用者向けの表示へ変換する。画面には内部の詳細を出さない（詳細はログに残す。秘密情報・認証情報・個人情報は出さない）
- クライアントのコードと設定（Web の app、Flutter の `lib/config/` など）は利用者の手元に配られるので、秘密情報（API キー・クライアントシークレットなど）を置かない。認可はサーバで判定し、クライアントでの判定は表示の制御に留める。利用者に見せてはいけないデータは、サーバの応答に含めない
- ルートのパスの定数（画面の遷移に使う）は、Web は `src/presentation/` に置く。Flutter は case study どおり `lib/routing/routes.dart` に置き、ui はこのファイルだけ参照してよい（このファイルは他の層を import しない）
- サーバの状態のキャッシュのライブラリが画面の層で動く形（React の hook など）のときは、取得を use-case（Web は interactor）経由にすれば、キャッシュを画面の層に置いてよい
- 既存のクライアントがこの構成でなければ、既存の構成と命名に合わせる。この構成へ移行するかは各リポジトリの Issue で決める
- 手本のリポジトリは無い。要るようになったら、バックエンドのテンプレートと同じく別のリポジトリとして作る

### Web（React の SPA）

レイヤーの名前はバックエンドに揃える。

| レイヤー | 置き場所 | 中身 | 依存してよい先 |
|---|---|---|---|
| model | `src/domain/model/` | エンティティ・値オブジェクト | なし |
| use-case | `src/domain/use-case/` | `interactor/`（ユースケース。1 ユースケース 1 ファイル）、`gateway/`（外部依存の抽象。1 外部依存 1 ファイル）、`errors/` | model |
| gateway | `src/gateway/<外部サービス名>/` | use-case の gateway の実装（API クライアント・ブラウザの保存領域など）、`errors/`。API の型からモデルへの変換と、サーバの状態の取得・キャッシュを含む | model・use-case |
| presentation | `src/presentation/` | 画面（`pages/`）、部品（`components/`）、画面の状態 | model・use-case の interactor と errors |
| app | `src/app/` | 起動、依存の組み立て（gateway の実装を生成して interactor に渡す）、ルーティングの登録 | すべて |

presentation は、interactor を受け取る口（Context と hook、または props）を自分の中に置き、値は app が渡す。presentation は app を import しない。

### モバイル（Flutter）

公式の case study（https://docs.flutter.dev/app-architecture/case-study ）のパッケージ構成と名前を基にする。外れるのは、use-case の層を必須にして `lib/domain/use_cases/` に置くことと、repository の抽象を `lib/domain/repositories/` に置くこと（case study は domain に抽象の置き場所を設けていない）の 2 つ。原則の gateway は repository に当たる。

| レイヤー | 置き場所 | 中身 | 依存してよい先 |
|---|---|---|---|
| model | `lib/domain/models/` | ドメインモデル | なし |
| use-case | `lib/domain/use_cases/` | ユースケース（1 ユースケース 1 ファイル）、エラー型（`lib/domain/use_cases/errors/`。エラー型は model だけに依存する） | model・repository の抽象 |
| repository の抽象 | `lib/domain/repositories/` | 外部依存の抽象（`abstract interface class`） | model・use-case のエラー型 |
| data | `lib/data/repositories/`・`lib/data/services/`・`lib/data/model/` | repository の実装、service（API・プラットフォーム・ローカルファイルのラッパー）、API のモデル。API のモデルからドメインモデルへの変換、data のエラーから use-case のエラー型への変換と、サーバの状態の取得・キャッシュは repository の実装で行う | model・repository の抽象・use-case のエラー型 |
| ui | `lib/ui/<feature>/view_models/`・`widgets/`（共通は `lib/ui/core/`） | view（widget）と view model。画面の状態は view model が持つ | model・use-case |
| 組み立て・起動 | `lib/main.dart`・`lib/config/`・`lib/routing/` | 起動、依存の組み立て（repository の実装を生成して use-case に渡す）、ルーティング | すべて |

補助の `lib/utils/` には、case study の Result・Command などを置く。環境ごとの起動（`main_<環境>.dart`）は組み立て・起動に含む。

### 認証情報と永続化したキャッシュ

Web と Flutter に共通。層の名前は上の 2 つの表のとおり。

- 認証情報（トークン・セッション）は、データの層（Web は gateway の実装、Flutter は data の repository の実装と service）が保持し、use-case 側の抽象（Web は `src/domain/use-case/gateway/`、Flutter は `lib/domain/repositories/`）越しに扱う。画面の層と use-case は認証情報の値を受け取らず、ログイン・ログアウト・認証済みかの操作だけを扱う（画面の層は use-case 越しに、use-case は抽象越しに）
- API クライアント（Web は gateway の実装、Flutter は data の service）は、use-case 側に置いた認証情報の抽象を受け取って要求に付ける（要求に付ける値を返す抽象は、データの層だけが呼び、use-case は呼ばない）。API クライアントは値の写しを持たず、要求のたびに抽象から取る。実装は app（組み立て・起動）が差し込む。Web は gateway の実装同士を直接 import しない
- 端末に保存してよい。保存するときはプラットフォームが保護する保存領域（他のアプリ・スクリプトから読めない領域）を使い、平文のファイルやスクリプトから読める領域に置かない（アプリ専用のディレクトリでも、平文で書かれる設定ストア・DB は平文のファイルに含む）。Web はスクリプトから読める保存領域（localStorage・sessionStorage など）に認証情報を置かず、保存するなら httpOnly の Cookie などスクリプトから読めない仕組みに任せるか、メモリだけに留める。Cookie に任せるときは、保持と要求への付与はブラウザが行い、データの層はログイン・ログアウトの要求を持つ。具体の領域・ライブラリと、Web でどちらにするかは「未定義の領域」のとおり hve-local に書く
- ログアウト・ユーザー切り替えのユースケース（use-case の層）が、認証情報・サーバの状態のキャッシュ・ローカルの保存の破棄を、それぞれを保持するデータの層に対する use-case 側の抽象のメソッド越しに呼ぶ。消す処理の実装は保持しているデータの層が持つ
- 画面の層で動くキャッシュ（取得を use-case 経由にして画面の層に置いた形）は、画面の層がログアウト・ユーザー切り替えのユースケースの完了に合わせて消す（use-case は画面の層を知らない）
