//! `/docs`（Swagger UI）と `/api-docs/openapi.json` の**配信経路**の検査（#616）。
//!
//! `tests/openapi.rs` と `tests/openapi_route_parity.rs` が見ているのは**仕様の生成側**で、
//! `scripts/check-vendored-swagger.sh` が見ているのは **`vendored` feature の在否**だけ。
//! ADR 0082（`docs/original-docs/0082-swagger-ui-vendored.md`）で Swagger UI を
//! `utoipa-swagger-ui-vendored` の埋め込み資産へ載せ替えたので、ここが**配信側の唯一の検査**になる。
//!
//! **何を捕まえるか**: 資産そのものの取り込み失敗（zip の展開失敗）は上流の build script が panic して
//! **コンパイル時に落ちる**ので、ここには来ない。このテストが押さえるのは **(a) 上流の版が上がった
//! ときの資産名・構造のドリフト**（`index.html` が読み込む資産の名前が変わる／欠ける）と、
//! **(b) `SwaggerUi` の配線ミス**（マウント先・spec URL・別の `ApiDoc` の混線）、そして
//! **(c) 描画元に外部オリジンが混ざる逆戻り**（`SwaggerUi::url("https://…")` 等。`vendored` feature の
//! 脱落は配信 HTML が同一なのでここでは検知できない——それは `check-vendored-swagger.sh` の担当）。
//! どれも資産が「在る」まま UI だけが壊れるので、**200 が返るだけでは足りない**——本文まで見る。
//!
//! Postgres 不要: `/docs/*` のハンドラは `web::Path` と `web::Data<Config>` しか、
//! `/api-docs/openapi.json` のハンドラは `web::Data<ApiDoc>` しか抽出しない。DB プールが要るのは
//! `configure_routes::<R, O, S>` のジェネリクス `R` を具象化するためだけで、接続は一度も張られない。
//! そのため api-server の多数派である `#[sqlx::test]` は採らない（配信経路と無関係な理由——Postgres
//! 未起動や migration 適用——でローカル実行が落ちるのを避ける）。先例は `openapi_route_parity.rs`。
//! プール URL に**到達不能なアドレスを意図的に置く**ことで、「DB を触らない」を偶然でなく
//! テストが強制する性質にしている（DB 依存が紛れ込んだ瞬間に落ちる）。
//!
//! `/docs`（末尾スラッシュ無し）が 404 である現状は**意図的に固定していない**——リダイレクトを足すか
//! 404 のままにするかは #619 で決める。ここで pin すると将来の変更を阻む側に回る。
//!
//! 前提: `/docs` と `/api-docs/openapi.json` は `/api` スコープの**外**にあり、現状は認証対象外
//! （`app::configure_routes` の doc を参照）。ここでの 200 は「無認証で配信されるべき」という要件では
//! なく現状の追認なので、docs を保護する変更を入れるときはこのテストも併せて更新する。

use std::collections::BTreeSet;
use std::time::Duration;

use actix_web::{App, http::header::CONTENT_TYPE, test as actix_test, web};
use sqlx::postgres::PgPoolOptions;
use utoipa::OpenApi;

use api_server::app::configure_routes;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_use_case::Interactor;
use rdb_gateway::PostgresRepository;
use rest_controller::openapi::ApiDoc;

type Repo = PostgresRepository;

/// **本文まで見る**資産の表。ここに無い参照も `index.html` から抽出して 200 だけは確認するので
/// （`refs` 参照）、この表は「深く見る対象」であって参照の網羅リストではない。
///
/// 下限は実測（`swagger-ui.css` 152KB / `index.css` 202B / bundle 1.42MB /
/// standalone-preset 230KB / initializer 423B）に対して、大きい資産は約 7 割、小さい資産は約 5 割。
/// 桁違いの欠損だけでなく途中で切れた配信も落とせる粒度にしてある（小さい資産で 7 割まで攻めると
/// 上流の些細な整形差で鳴るので緩めている）。上流の版差でこの範囲を割るなら実測を見直す。
const ASSETS: [Asset; 5] = [
    Asset {
        file: "swagger-ui.css",
        content_type_fragment: "css",
        min_bytes: 100_000,
        signature: Some(".swagger-ui"),
    },
    Asset {
        file: "index.css",
        content_type_fragment: "css",
        min_bytes: 100,
        signature: Some("box-sizing"),
    },
    Asset {
        file: "swagger-ui-bundle.js",
        content_type_fragment: "javascript",
        min_bytes: 1_000_000,
        signature: Some("SwaggerUIBundle"),
    },
    Asset {
        file: "swagger-ui-standalone-preset.js",
        content_type_fragment: "javascript",
        min_bytes: 150_000,
        signature: Some("SwaggerUIStandalonePreset"),
    },
    Asset {
        file: "swagger-initializer.js",
        content_type_fragment: "javascript",
        min_bytes: 200,
        signature: Some("SwaggerUIBundle"),
    },
];

struct Asset {
    /// `index.html` から参照される名前であり、`/docs/<file>` で配信される名前でもある。
    file: &'static str,
    /// `Content-Type` に含まれるべき部分文字列（MIME 型そのものではない。`mime_guess` の
    /// 出力が版で揺れても通るよう、意図的に緩く見る）。
    content_type_fragment: &'static str,
    /// 実体のバイト数がこれを**超える**こと。
    min_bytes: usize,
    /// 本文に含まれるべきシグネチャ（サイズだけだと別の大きな資産が返っても通るため）。
    signature: Option<&'static str>,
}

/// `index.html` の `src=` / `href=` から参照先を抽出し、`(ローカル, 外部)` に分けて返す。
///
/// 固定リスト（`ASSETS`）だけを回すと、**上流が資産を増やしたとき**（`index.html` は参照するが
/// 配信されない）に気づけない。実際 favicon 2 本は `ASSETS` に無い。抽出側を実本文から取ることで
/// additive なドリフトも 200 の確認までは届く。
///
/// 外部判定を属性値に限定するのが要点——本文全体への `contains("http://")` だと、上流がコメントや
/// `meta` に URL を書いただけで「外部オリジンを参照している」と誤検知する。
///
/// ローカル側は `/docs/` を基点とした名前へ正規化する（`./x` と `/x` を同じ `x` に寄せる）。
/// 正規化しないと `/docs//x` を叩いて「資産構成が変わった疑い」という方向違いのメッセージで落ちる。
fn refs(html: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let (mut local, mut external) = (BTreeSet::new(), BTreeSet::new());
    for attr in ["src=\"", "href=\"", "src='", "href='"] {
        let quote = attr.chars().last().expect("attr ends with a quote");
        for part in html.split(attr).skip(1) {
            let value = part.split(quote).next().unwrap_or("");
            if value.is_empty() || value.starts_with('#') {
                continue;
            }
            // `//host/x`（プロトコル相対）と `scheme:...`（http/https/data/mailto 等）は外部扱い。
            // スキーム判定は最初の `/` より前に `:` があるかで見る。
            let is_scheme_relative = value.starts_with("//");
            let has_scheme = value
                .split('/')
                .next()
                .is_some_and(|first| first.contains(':'));
            if is_scheme_relative || has_scheme {
                external.insert(value.to_string());
                continue;
            }
            local.insert(
                value
                    .trim_start_matches("./")
                    .trim_start_matches('/')
                    .to_string(),
            );
        }
    }
    (local, external)
}

/// テスト用 actix App を組み立てる。`init_service` の戻り値は名前で書けない型なのでマクロにする
/// （`tests/api.rs` の `build_service!` と同じ理由）。
///
/// 同名マクロは `api.rs` / `session.rs` / `prediction.rs` にもあり、これが 4 つ目の写しになる。
/// `tests/common/` への集約は #620 で扱う（5 つ目を足す前に返す）。**この版だけ「到達不能プール
/// ＋1 秒 timeout」という別契約**なので、共通化するときはプールを引数化すること——素朴に実 DB
/// プールへ寄せると「DB を触らないことの強制」が静かに失われる。
macro_rules! build_service {
    () => {{
        // 到達不能なアドレスへの遅延接続プール（上のモジュール doc を参照）。
        // `acquire_timeout` を縮めるのは、DB 依存が紛れ込んだときに既定の 30 秒待たされて
        // 「CI でハングした」に見えるのを避けるため（fail-fast にして原因を読みやすくする）。
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_secs(1))
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .expect("build lazy pool");
        let interactor = web::Data::new(Interactor::new(PostgresRepository::new(pool)));
        actix_test::init_service(
            App::new()
                .app_data(interactor)
                .configure(configure_routes::<Repo, UreqNetkeibaScraper, UreqNetkeibaScraper>),
        )
        .await
    }};
}

/// `GET <uri>` を投げて `(status, content-type, body)` を返す。
///
/// `tests/prediction.rs` にも `get!` があるが、あちらは future を返して呼び出し側で await する
/// 別契約。テストバイナリが別なので衝突はしないが、読み違えを避けるため名前を分けている。
macro_rules! get_parts {
    ($app:expr, $uri:expr) => {{
        let req = actix_test::TestRequest::get().uri($uri).to_request();
        let resp = actix_test::call_service($app, req).await;
        let status = resp.status();
        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = actix_test::read_body(resp).await;
        (status, content_type, body)
    }};
}

/// 失敗メッセージ用に本文の先頭を切り出す（HTTP の HEAD とは無関係）。**文字境界で切る**——バイト境界で切ると、
/// 非 ASCII が配信された場合（＝まさにこの診断が要る場面）にスライスが panic して
/// 本来のメッセージが消える。
fn body_excerpt(body: &[u8]) -> String {
    String::from_utf8_lossy(body).chars().take(500).collect()
}

/// Swagger UI の殻（`index.html`）が配信され、UI の組み立てに要る資産をすべて参照していること。
///
/// 末尾スラッシュ付きの `/docs/` も同じ `index.html` にフォールバックする（人間が開く URL はこちら）。
/// 資産の 200 確認は本文が同一であることを確かめたうえで 1 周だけ回す（両 URI で回しても得られる
/// 情報は増えず、1.4MB の bundle を二度取ることになる）。
#[actix_web::test]
async fn docs_index_serves_swagger_ui_shell() {
    let app = build_service!();

    let mut shells = Vec::new();
    for uri in ["/docs/index.html", "/docs/"] {
        let (status, content_type, body) = get_parts!(&app, uri);
        assert_eq!(
            status, 200,
            "{uri} が 200 を返さない: 埋め込み資産の取り込みを疑う"
        );
        assert!(
            content_type.contains("text/html"),
            "{uri} の Content-Type が HTML でない: {content_type}"
        );
        shells.push(body);
    }
    assert_eq!(
        shells[0], shells[1],
        "/docs/ が index.html にフォールバックしていない（上流のフォールバック実装が変わった疑い）"
    );

    let body = &shells[0];
    let html = String::from_utf8_lossy(body);
    // 属性の整形差（引用符・属性順・空白）で偽陽性にならないよう、UI のマウント先の
    // 識別子だけを見る。UI の描画は `id="swagger-ui"` の要素があることに依存している。
    assert!(
        html.contains(r#"id="swagger-ui""#),
        "殻の本文に Swagger UI のマウント先が無い（配信されたのは別の何か）:\n{}",
        body_excerpt(body)
    );

    let (local, external) = refs(&html);
    // 深く見る資産（`ASSETS`）が殻から参照されていること。抽出結果に対する包含で見るので、
    // **抽出そのものが空振りしていれば同時に落ちる**（本文が変わって抽出が 0 件になっても
    // 下のループが素通りする、という壊れ方を防ぐ）。
    for asset in ASSETS {
        assert!(
            local.contains(asset.file),
            "殻が {} を参照していない（抽出できた参照: {local:?}）: 上流の資産構成が変わった疑い\n{}",
            asset.file,
            body_excerpt(body)
        );
    }

    // 殻が参照する**すべての**ローカル資産が配信されること（favicon 等、表に無いものも含む）。
    // 上流が資産を増やしたときの取り逃がしを防ぐ。
    for name in local {
        let asset_uri = format!("/docs/{name}");
        let (status, _, _) = get_parts!(&app, &asset_uri);
        assert_eq!(
            status, 200,
            "殻が参照する {asset_uri} が配信されていない: 上流の資産構成が変わった疑い"
        );
    }

    // 殻が外部オリジンを読みに行かないこと。上流資産が変わらない限り発火しない保険で、
    // 外部取得の復活を実際に捕まえるのは下の initializer 側のアサーション（配線はそこに出る）。
    assert!(
        external.is_empty(),
        "殻が外部オリジンを参照している（vendored の意味が失われる）: {external:?}"
    );
}

/// UI の描画元が**この API の spec** を指していること。
///
/// `swagger-initializer.js` は配信時に `{{config}}` が spec URL を含む JSON へ置換される。
/// 上流の既定は petstore を指すので、置換が壊れると UI は**他人の spec を読む**。
#[actix_web::test]
async fn swagger_initializer_points_at_our_openapi_json() {
    let app = build_service!();

    let (status, content_type, body) = get_parts!(&app, "/docs/swagger-initializer.js");
    assert_eq!(status, 200, "swagger-initializer.js が配信されていない");
    assert!(
        content_type.contains("javascript"),
        "swagger-initializer.js の Content-Type が JS でない: {content_type}"
    );

    let js = String::from_utf8_lossy(&body);
    assert!(
        js.contains("/api-docs/openapi.json"),
        "UI の描画元が /api-docs/openapi.json を指していない（SwaggerUi::url の配線を疑う）:\n{}",
        body_excerpt(&body)
    );
    assert!(
        !js.contains("petstore"),
        "UI の描画元に上流既定の petstore が残っている（{{config}} の置換が壊れた疑い）:\n{}",
        body_excerpt(&body)
    );
    // ADR 0082 のランタイム側の対応物は**ここ**が本命。外部取得の復活は
    // `SwaggerUi::url("https://…")` や `.urls([自 spec, 外部 spec])` としてアプリ側の配線に現れ、
    // それが initializer の config に載る（殻の `index.html` は素通し配信なので変わらない）。
    // `://` はスキーム付き URL、`"//` は JSON 値としてのプロトコル相対 URL を捕まえる
    // （素の `//` は JS のコメント記法に当たるので使えない）。
    assert!(
        !js.contains("://") && !js.contains("\"//"),
        "UI の描画元に外部オリジンが混ざっている（vendored の意味が失われる）:\n{}",
        body_excerpt(&body)
    );
}

/// `index.html` が参照する資産が実体を伴って配信されること（参照側は上のテストが見る）。
#[actix_web::test]
async fn swagger_ui_assets_have_substance() {
    let app = build_service!();

    for asset in ASSETS {
        let uri = format!("/docs/{}", asset.file);
        let (status, content_type, body) = get_parts!(&app, &uri);
        assert_eq!(status, 200, "{uri} が配信されていない");
        assert!(
            content_type.contains(asset.content_type_fragment),
            "{uri} の Content-Type が {} でない: {content_type}",
            asset.content_type_fragment
        );
        assert!(
            body.len() > asset.min_bytes,
            "{uri} が小さすぎる（{} bytes <= {}）: 埋め込み資産が壊れている疑い",
            body.len(),
            asset.min_bytes
        );
        if let Some(signature) = asset.signature {
            assert!(
                String::from_utf8_lossy(&body).contains(signature),
                "{uri} に {signature} が無い（別の資産が配信された疑い）:\n{}",
                body_excerpt(&body)
            );
        }
    }
}

/// `/api-docs/openapi.json` が **この API の `ApiDoc`** として配信されること。
///
/// `paths` 非空だけだと「別の `ApiDoc` が `SwaggerUi::url` に配線された」退行を取り逃がす
/// （`openapi.rs` / `openapi_route_parity.rs` は生成側しか見ないので、配信物との照合はここが唯一）。
#[actix_web::test]
async fn openapi_json_is_served_from_our_apidoc() {
    let app = build_service!();

    let (status, content_type, body) = get_parts!(&app, "/api-docs/openapi.json");
    assert_eq!(status, 200, "/api-docs/openapi.json が 200 を返さない");
    assert!(
        content_type.contains("application/json"),
        "/api-docs/openapi.json の Content-Type が JSON でない: {content_type}"
    );

    let served: serde_json::Value =
        serde_json::from_slice(&body).expect("/api-docs/openapi.json が JSON として読めない");
    let expected = serde_json::to_value(ApiDoc::openapi()).expect("ApiDoc をシリアライズできない");

    // 先に paths のキー集合だけ突き合わせる。丸ごと assert_eq! すると失敗時に spec 2 つ分が
    // ログへ流れて差分が埋もれるため、まず「どのパスが違うか」を名指しで出す。
    let path_keys = |v: &serde_json::Value| -> BTreeSet<String> {
        v.get("paths")
            .and_then(|p| p.as_object())
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    };
    let (served_paths, expected_paths) = (path_keys(&served), path_keys(&expected));
    assert!(
        !served_paths.is_empty(),
        "paths が空: UI は \"No operations defined in spec!\" になる"
    );
    assert_eq!(
        served_paths,
        expected_paths,
        "配信された spec の paths が ApiDoc::openapi() と一致しない（配信のみ: {:?} / 生成のみ: {:?}）",
        served_paths.difference(&expected_paths).collect::<Vec<_>>(),
        expected_paths.difference(&served_paths).collect::<Vec<_>>(),
    );
    if served != expected {
        // paths が一致してなお違うなら info / components / tags 側。どのキーで割れたかまでは
        // 名指しする（本文は出さない——spec 2 つ分がログへ流れると差分が埋もれる）。
        let top_keys = |v: &serde_json::Value| -> BTreeSet<String> {
            v.as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default()
        };
        let differing: Vec<String> = top_keys(&served)
            .union(&top_keys(&expected))
            .filter(|k| served.get(*k) != expected.get(*k))
            .cloned()
            .collect();
        panic!(
            "配信された spec が ApiDoc::openapi() と一致しない（差分のあるトップレベルキー: {differing:?}）: SwaggerUi::url の配線を疑う"
        );
    }
}

/// 負のコントロール: 存在しない資産は 404。
///
/// 上の 200 系アサーションが「どの URI でも同じ本文を返す」壊れ方をしていないことの確認。
/// なお SwaggerUi のマウントが丸ごと消えても actix の既定 404 で成立するので、**これ単体では
/// 配信の健全性を担保しない**（担保しているのは 200 系が本文まで見ていること）。
#[actix_web::test]
async fn unknown_docs_asset_is_not_found() {
    let app = build_service!();

    let (status, _, _) = get_parts!(&app, "/docs/__definitely_not_an_asset__");
    assert_eq!(
        status, 404,
        "存在しない資産が 404 以外を返した: 配信判定が常に成功する壊れ方をしている"
    );
}
