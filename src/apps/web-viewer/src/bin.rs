use actix_web::{App, HttpResponse, HttpServer, Responder, web};
use tracing_subscriber::EnvFilter;

mod config;
mod render;
mod tree;

use config::PadConfig;

// アセットはバイナリに埋め込み、実ファイル配置に依存せず単体で配信する。
const INDEX_HTML: &str = include_str!("../assets/index.html");
const APP_CSS: &str = include_str!("../assets/app.css");
const APP_JS: &str = include_str!("../assets/app.js");

async fn index() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(INDEX_HTML)
}

async fn app_css() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/css; charset=utf-8")
        .body(APP_CSS)
}

async fn app_js() -> impl Responder {
    HttpResponse::Ok()
        .content_type("application/javascript; charset=utf-8")
        .body(APP_JS)
}

// ブラウザの自動リクエスト。コンテンツは無いので 204 を返してコンソールエラーを避ける。
async fn favicon() -> impl Responder {
    HttpResponse::NoContent().finish()
}

async fn api_tree(cfg: web::Data<PadConfig>) -> impl Responder {
    HttpResponse::Ok().json(tree::build_tree(&cfg.pad_dir))
}

#[derive(serde::Deserialize)]
struct DocQuery {
    path: String,
}

async fn api_doc(cfg: web::Data<PadConfig>, q: web::Query<DocQuery>) -> impl Responder {
    match render::render_doc(&cfg.pad_dir, &q.path) {
        Ok(html) => HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(html),
        Err(render::RenderError::NotFound) => HttpResponse::NotFound().body("not found"),
        Err(render::RenderError::Server) => {
            HttpResponse::InternalServerError().body("server error")
        }
        Err(render::RenderError::Invalid) => HttpResponse::BadRequest().body("bad request"),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = PadConfig::from_env();
    let port = cfg.port;
    tracing::info!(
        "pad-web listening on http://localhost:{port}  (pad_dir = {})",
        cfg.pad_dir.display()
    );

    let data = web::Data::new(cfg);
    HttpServer::new(move || {
        App::new()
            .app_data(data.clone())
            .route("/", web::get().to(index))
            .route("/favicon.ico", web::get().to(favicon))
            .route("/static/app.css", web::get().to(app_css))
            .route("/static/app.js", web::get().to(app_js))
            .route("/api/tree", web::get().to(api_tree))
            .route("/api/doc", web::get().to(api_doc))
    })
    .bind(("127.0.0.1", port))?
    .run()
    .await
}
