//! OpenAPI 仕様のスナップショットテスト（#33）。
//!
//! コードファースト（utoipa）で生成した OpenAPI ドキュメントが、コミット済みの
//! `docs/api/openapi.json` と一致することを検証する。スキーマ変更時にコミットの更新を
//! 忘れると CI（このテスト）が落ちる。Postgres 不要。
//!
//! 再生成: `UPDATE_OPENAPI=1 cargo test -p api-server --test openapi`

use rest_controller::openapi::ApiDoc;
use utoipa::OpenApi;

/// リポジトリ直下の生成物パス（テストの CWD はクレートルートなので manifest 基準で解決）。
const OPENAPI_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../docs/api/openapi.json"
);

/// 生成は serde の構造体定義順 + pretty 整形のみで安定化する（フィールド順入れ替えに依存しない）。
fn generate() -> String {
    serde_json::to_string_pretty(&ApiDoc::openapi()).expect("serialize OpenAPI to JSON")
}

/// 注意: このテストは「コミット済み JSON が生成結果と一致するか」のみを検証する。
/// 生成結果も期待値も同じ `ApiDoc` 由来のため、handler/schema を `openapi.rs` の
/// `paths(...)` / `components(...)` 列挙に追加し忘れた場合（＝ docs からの欠落）は検知できない。
/// 列挙漏れは PR レビューと、エンドポイント追加時の openapi.json 差分で気づく運用とする。
#[test]
fn openapi_snapshot_is_up_to_date() {
    let generated = generate();

    if std::env::var_os("UPDATE_OPENAPI").is_some() {
        // git フレンドリに末尾改行を付けてコミットする。
        std::fs::write(OPENAPI_PATH, format!("{generated}\n"))
            .expect("write docs/api/openapi.json");
        return;
    }

    let committed = std::fs::read_to_string(OPENAPI_PATH).unwrap_or_else(|e| {
        panic!(
            "read {OPENAPI_PATH}: {e}\n\
             生成し直す場合: UPDATE_OPENAPI=1 cargo test -p api-server --test openapi"
        )
    });

    // 末尾改行の有無で偽陽性 fail しないよう、両者を trim_end して比較する。
    assert_eq!(
        generated.trim_end(),
        committed.trim_end(),
        "docs/api/openapi.json がコード生成結果と乖離しています。\n\
         再生成: UPDATE_OPENAPI=1 cargo test -p api-server --test openapi"
    );
}
