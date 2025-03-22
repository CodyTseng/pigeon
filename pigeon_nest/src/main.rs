use std::{env, net::SocketAddr};

use dotenv::dotenv;
use pigeon_nest::CONFIG;
use pigeon_nest::create_routes;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    dotenv().ok();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let routes = create_routes();
    let addr = SocketAddr::from(([0, 0, 0, 0], CONFIG.port));

    info!("Listening on {}", addr);
    warp::serve(routes).run(addr).await;
}
