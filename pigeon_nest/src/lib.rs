mod client;
mod config;
mod node;
mod nostr_event;
mod pool;
mod relay;
mod utils;

use std::sync::Arc;

use axum::{
    Extension, Router,
    extract::{Path, ws::WebSocketUpgrade},
    response::Response,
    routing::{any, get},
};
pub use config::CONFIG;
use pool::Pool;

pub fn create_app() -> Router {
    let pool = Arc::new(Pool::new());

    Router::new()
        .route("/", get(index_handler))
        .route("/register", any(relay_registration_handler))
        .route("/{node_id}", any(client_connection_handler))
        .layer(Extension(pool))
}

async fn index_handler(state: Extension<Arc<Pool>>) -> String {
    let pool = state.0.clone();
    let size = pool.get_pool_size();
    format!("Proxying for {} relays", size)
}

async fn relay_registration_handler(state: Extension<Arc<Pool>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(async move |socket| {
        let pool = state.0.clone();
        pool.register(socket).await;
    })
}

async fn client_connection_handler(
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
