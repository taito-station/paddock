use actix_web::{App, HttpServer, web};
use netkeiba_scraper::UreqNetkeibaScraper;
use odds_scraper::UreqOddsScraper;
use rdb_gateway::PostgresRepository;

use api_server::app;
use api_server::setup::{self, UnusedFetcher, UnusedParser};

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    let s = setup::build().await?;
    let addr = s.server_addr.clone();
    let interactor = web::Data::new(s.interactor);
    let odds = web::Data::new(s.odds);
    let settle = web::Data::new(s.settle);

    tracing::info!("paddock api-server listening on http://{addr} (docs: /docs)");

    HttpServer::new(move || {
        App::new()
            .app_data(interactor.clone())
            .app_data(odds.clone())
            .app_data(settle.clone())
            .configure(
                app::configure_routes::<
                    PostgresRepository,
                    UnusedParser,
                    UnusedFetcher,
                    UreqOddsScraper,
                    UreqNetkeibaScraper,
                >,
            )
    })
    .bind(&addr)?
    .run()
    .await?;

    Ok(())
}
