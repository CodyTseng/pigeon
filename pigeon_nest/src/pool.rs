use std::sync::Arc;

use dashmap::DashMap;
use tracing::info;
use warp::filters::ws::WebSocket;

use crate::node::Node;

pub struct Pool {
    nodes: Arc<DashMap<String, Arc<Node>>>,
}

impl Pool {
    pub fn new() -> Self {
        let pool = Self {
            nodes: Arc::new(DashMap::new()),
        };
        pool.cleanup();
        pool
    }

    pub async fn register(&self, socket: WebSocket) {
        let node = match Node::new(socket).await {
            Some(node) => node,
            None => return,
        };
        let id = node.id();

        if let Some(existing_node) = self.nodes.get(&id) {
            existing_node.close().await;
        }

        self.nodes.insert(id.clone(), Arc::new(node));
        info!("[POOL] + NODE:{} (total nodes: {})", id, self.nodes.len());
    }

    pub fn get_node(&self, id: &str) -> Option<Arc<Node>> {
        self.nodes.get(id).map(|node| Arc::clone(node.value()))
    }

    pub fn get_pool_size(&self) -> usize {
        self.nodes.len()
    }

    fn cleanup(&self) {
        let nodes = Arc::clone(&self.nodes);

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;

                let to_remove = nodes
                    .iter()
                    .filter_map(|entry| {
                        let node = entry.value();
                        if !node.is_active() || node.seconds_since_last_active() > 600 {
                            return Some(entry.key().clone());
                        }
                        None
                    })
                    .collect::<Vec<String>>();

                for id in to_remove {
                    if let Some((_, node)) = nodes.remove(&id) {
                        node.close().await;
                        info!("[POOL] - NODE:{} (total nodes: {})", id, nodes.len());
                    }
                }
            }
        });
    }
}
