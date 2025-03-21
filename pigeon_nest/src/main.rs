use std::{env, net::SocketAddr};

use dotenv::dotenv;
use pigeon_nest::CONFIG;
use pigeon_nest::create_app;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let app = create_app();
    let addr = SocketAddr::from(([0, 0, 0, 0], CONFIG.port));
    let listener = TcpListener::bind(addr).await?;

    info!("Listening on {}", addr);
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}
