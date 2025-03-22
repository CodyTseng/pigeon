mod client;
mod config;
mod node;
mod nostr_event;
mod pool;
mod relay;
mod utils;

use std::sync::Arc;

pub use config::CONFIG;
use pool::Pool;
use warp::Filter;

pub fn create_routes() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let pool = Arc::new(Pool::new());
    let pool_filter = warp::any().map(move || Arc::clone(&pool));

    let index_route = warp::path::end()
        .and(pool_filter.clone())
        .map(|pool: Arc<Pool>| {
            let size = pool.get_pool_size();
            format!("Proxying for {} relays", size)
        });

    let connect_route = warp::path!(String)
        .and(warp::ws())
        .and(pool_filter.clone())
        .map(|node_id: String, ws: warp::ws::Ws, pool: Arc<Pool>| {
            ws.on_upgrade(move |socket| async move {
                let node = match pool.get_node(&node_id) {
                    Some(node) => node,
                    None => return,
                };
                if node.is_active() {
                    node.add_client(socket);
                }
            })
        });

    let relay_info_route =
        warp::path!(String)
            .and(pool_filter.clone())
            .map(
                |node_id: String, pool: Arc<Pool>| match pool.get_node(&node_id) {
                    Some(node) => warp::reply::with_status(
                        node.get_relay_info().to_string(),
                        warp::http::StatusCode::OK,
                    ),
                    None => warp::reply::with_status(
                        "Relay not found".to_string(),
                        warp::http::StatusCode::NOT_FOUND,
                    ),
                },
            );

    let register_route = warp::path("register")
        .and(warp::ws())
        .and(pool_filter.clone())
        .map(|ws: warp::ws::Ws, pool: Arc<Pool>| {
            ws.on_upgrade(move |socket| async move {
                pool.register(socket).await;
            })
        });

    index_route
        .or(register_route)
        .or(connect_route)
        .or(relay_info_route)
        .with(warp::cors().allow_any_origin())
}
