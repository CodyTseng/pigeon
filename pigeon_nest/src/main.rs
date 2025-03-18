mod client;
mod config;
mod node;
mod nostr_event;
mod pool;
mod utils;

use std::{env, net::SocketAddr, sync::Arc};

use axum::{
    Extension, Router,
    extract::{Path, ws::WebSocketUpgrade},
    response::Response,
    routing::{any, get},
};
use config::CONFIG;
use dotenv::dotenv;
use pool::Pool;
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

    let pool = Arc::new(Pool::new());

    let app = Router::new()
        .route("/", get(handle_index))
        .route("/register", any(handle_relay_registration))
        .route("/{node_id}", any(handle_client_connection))
        .layer(Extension(pool));

    let addr = SocketAddr::from(([0, 0, 0, 0], CONFIG.port));
    let listener = TcpListener::bind(addr).await?;

    info!("Listening on {}", addr);
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

async fn handle_index(state: Extension<Arc<Pool>>) -> String {
    let pool = state.0.clone();
    let size = pool.get_pool_size();
    format!("Proxying for {} relays", size)
}

async fn handle_relay_registration(state: Extension<Arc<Pool>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(async move |socket| {
        let pool = state.0.clone();
        pool.register(socket).await;
    })
}

async fn handle_client_connection(
    state: Extension<Arc<Pool>>,
    Path(node_id): Path<String>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(async move |socket| {
        let pool = state.0.clone();
        let node = match pool.get_node(&node_id) {
            Some(node) => node,
            None => return,
        };
        if node.is_active() {
            node.add_client(socket);
        }
    })
}
