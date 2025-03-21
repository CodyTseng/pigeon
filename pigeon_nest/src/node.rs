use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use serde_json::json;
use tokio::sync::{mpsc, mpsc::Receiver, mpsc::Sender};
use tracing::info;

use crate::client::Client;
use crate::nostr_event::NostrEvent;
use crate::relay::Relay;

pub struct Node {
    id: String,
    relay: Arc<Relay>,
    clients: Arc<DashMap<String, Arc<Client>>>,
    client_to_node_tx: Sender<(String, Message)>,
    in_progress_events: Arc<DashMap<String, Vec<String>>>,
}

impl Node {
    pub async fn new(ws: WebSocket) -> Option<Self> {
        let (relay_to_node_tx, relay_to_node_rx) = mpsc::channel::<Message>(32);
        let relay = match Relay::new(ws, relay_to_node_tx).await {
            Some(relay) => relay,
            None => return None,
        };

        let (client_to_node_tx, client_to_node_rx) = mpsc::channel::<(String, Message)>(32);
        let node = Self {
            id: relay.id(),
            relay: Arc::new(relay),
            client_to_node_tx,
            clients: Arc::new(DashMap::new()),
            in_progress_events: Arc::new(DashMap::new()),
        };
        node.handle_relay_messages(relay_to_node_rx);
        node.handle_client_messages(client_to_node_rx);
        node.cleanup();
        Some(node)
    }

    pub fn id(&self) -> String {
        self.id.clone()
    }

    pub fn is_active(&self) -> bool {
        self.relay.is_active()
    }

    pub fn seconds_since_last_active(&self) -> u64 {
        self.relay.seconds_since_last_active()
    }

    pub async fn close(&self) {
        let _ = self.relay.close().await;
    }

    pub fn add_client(&self, ws: WebSocket) {
        let clients = Arc::clone(&self.clients);
        let client_id = loop {
            let client_id = rand::random::<u16>().to_string();
            if !clients.contains_key(&client_id) {
                break client_id;
            }
        };

        let client = Client::new(client_id.clone(), ws, self.client_to_node_tx.clone());
        clients.insert(client_id.clone(), Arc::new(client));
        info!(
            "[NODE:{}] + CLIENT:{} (total clients: {})",
            self.id,
            client_id,
            clients.len()
        );
    }

    fn handle_relay_messages(&self, mut receiver: Receiver<Message>) {
        let clients = Arc::clone(&self.clients);
        let in_progress_events = Arc::clone(&self.in_progress_events);

        tokio::spawn(async move {
            while let Some(Message::Text(text)) = receiver.recv().await {
                let parsed: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(parsed) => parsed,
                    Err(_) => continue,
                };
                if !parsed.is_array() {
                    continue;
                }

                let arr = match parsed.as_array() {
                    Some(arr) => arr,
                    None => continue,
                };
                if arr.len() < 2 {
                    continue;
                }

                let msg_type = &arr[0];
                if msg_type == "EVENT" || msg_type == "EOSE" || msg_type == "CLOSED" {
                    let sub_id = match arr[1].as_str() {
                        Some(sub_id) => sub_id,
                        None => continue,
                    };
                    let (client_id, raw_sub_id) = match sub_id.split_once(':') {
                        Some(arr) => arr,
                        None => continue,
                    };

                    if let Some(client) = clients.get(client_id) {
                        let mut new_arr = Vec::new();
                        new_arr.push(msg_type.clone());
                        // Restore the original subscription ID
                        new_arr.push(json!(raw_sub_id));

                        // Add all remaining elements from the original array
                        new_arr.extend(arr.iter().skip(2).cloned());

                        client
                            .send(Message::Text(json!(new_arr).to_string().into()))
                            .await;
                    }
                } else if msg_type == "OK" {
                    let event_id = match arr[1].as_str() {
                        Some(event_id) => event_id,
                        None => continue,
                    };

                    if let Some((_, client_ids)) = in_progress_events.remove(event_id) {
                        for client_id in client_ids {
                            if let Some(client) = clients.get(&client_id) {
                                client.send(Message::Text(text.clone())).await;
                            }
                        }
                    }
                }
            }
        });
    }

    fn handle_client_messages(&self, mut receiver: Receiver<(String, Message)>) {
        let in_progress_events = Arc::clone(&self.in_progress_events);
        let relay = Arc::clone(&self.relay);

        tokio::spawn(async move {
            while let Some((client_id, Message::Text(text))) = receiver.recv().await {
                let parsed: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(parsed) => parsed,
                    Err(_) => continue,
                };
                if !parsed.is_array() {
                    continue;
                }

                let arr = match parsed.as_array() {
                    Some(arr) => arr,
                    None => continue,
                };
                if arr.len() < 2 {
                    continue;
                }

                let msg_type = &arr[0];
                if msg_type == "REQ" || msg_type == "CLOSE" {
                    let sub_id = match arr[1].as_str() {
                        Some(sub_id) => sub_id,
                        None => continue,
                    };

                    let mut new_arr = Vec::new();
                    new_arr.push(json!(msg_type.clone()));
                    // Add client ID to the subscription ID
                    new_arr.push(json!(format!("{}:{}", client_id, sub_id)));

                    // Add all remaining elements from the original array
                    new_arr.extend(arr.iter().skip(2).cloned());

                    let _ = relay
                        .send(Message::Text(json!(new_arr).to_string().into()))
                        .await;
                } else if msg_type == "EVENT" {
                    let event = match NostrEvent::from_value(&arr[1]) {
                        Ok(event) => event,
                        Err(_) => continue,
                    };
                    in_progress_events
                        .entry(event.id().to_string())
                        .or_insert_with(Vec::new)
                        .push(client_id);

                    let _ = relay.send(Message::Text(text)).await;
                }
            }
        });
    }

    fn cleanup(&self) {
        let node_id = self.id.clone();
        let clients = Arc::clone(&self.clients);

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;

                let to_remove = clients
                    .iter()
                    .filter_map(|entry| {
                        let client = entry.value();
                        if !client.is_active() || client.seconds_since_last_active() > 180 {
                            return Some(entry.key().clone());
                        }
                        None
                    })
                    .collect::<Vec<String>>();

                for id in to_remove {
                    if let Some((client_id, client)) = clients.remove(&id) {
                        client.close().await;
                        info!(
                            "[NODE:{}] - CLIENT:{} (total clients: {})",
                            node_id,
                            client_id,
                            clients.len()
                        );
                    }
                }
            }
        });
    }
}
