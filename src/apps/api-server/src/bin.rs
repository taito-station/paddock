use actix_web::{App, HttpServer, web};

use api_server::app;
use api_server::setup::{self, UnusedFetcher, UnusedParser};

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    let s = setup::build().await?;
    let addr = s.server_addr.clone();
    let data = web::Data::new(s.interactor);

    tracing::info!("paddock api-server listening on http://{addr} (docs: /docs)");

    HttpServer::new(move || {
        App::new().app_data(data.clone()).configure(
            app::configure_routes::<rdb_gateway::PostgresRepository, UnusedParser, UnusedFetcher>,
        )
    })
    .bind(&addr)?
    .run()
    .await?;

    Ok(())
}
