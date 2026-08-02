use actix_web::{App, HttpServer, web};
use netkeiba_scraper::UreqNetkeibaScraper;
use rdb_gateway::PostgresRepository;

use api_server::app;
use api_server::setup;

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    let s = setup::build().await?;
    let addr = s.server_addr.clone();
    let interactor = web::Data::new(s.interactor);
    let odds = web::Data::new(s.odds);
    let results = web::Data::new(s.results);

    tracing::info!(
        "paddock api-server listening on http://{addr} (docs: /docs) — git {} built {} (#570: /api/health で世代確認)",
        rest_controller::build_info::GIT_SHA,
        rest_controller::build_info::build_time_rfc3339(),
    );

    HttpServer::new(move || {
        App::new()
            .app_data(interactor.clone())
            .app_data(odds.clone())
            .app_data(results.clone())
            .configure(
                app::configure_routes::<
                    PostgresRepository,
                    UreqNetkeibaScraper,
                    UreqNetkeibaScraper,
                >,
            )
    })
    .bind(&addr)?
    .run()
    .await?;

    Ok(())
}
