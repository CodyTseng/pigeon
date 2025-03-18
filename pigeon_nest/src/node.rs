use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use futures::SinkExt;
use futures::stream::{SplitSink, SplitStream, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, mpsc::Receiver, mpsc::Sender};
use tokio::time::timeout;
use tracing::info;

use crate::client::Client;
use crate::config::CONFIG;
use crate::nostr_event::NostrEvent;
use crate::utils::{extract_endpoint_from_url, unix_timestamp};

enum RelaySenderCommand {
    Send(Message),
    Close,
}

pub struct Node {
    id: String,
    is_active: Arc<AtomicBool>,
    last_active: Arc<AtomicU64>,
    relay_sender: Sender<RelaySenderCommand>,
    proxy_sender: Sender<(String, Message)>,
    clients: Arc<DashMap<String, Arc<Client>>>,
    in_progress_events: Arc<DashMap<String, Vec<String>>>,
}

impl Node {
    pub async fn new(ws: WebSocket) -> Option<Self> {
        let (mut sender, mut receiver) = ws.split();

        let challenge = format!("{:x}", rand::random::<u128>());
        let auth_request = json!(["AUTH", challenge]);
        if sender
            .send(Message::Text(auth_request.to_string().into()))
            .await
            .is_err()
        {
            return None;
        }
        let auth_message = match timeout(std::time::Duration::from_secs(10), receiver.next()).await
        {
            Ok(Some(msg)) => msg,
            _ => return None,
        };

        let event = match auth_message {
            Ok(Message::Text(auth_message)) => {
                if let Some(event) = check_auth_message(&auth_message, &challenge) {
                    event
                } else {
                    return None;
                }
            }
            _ => return None,
        };

        sender
            .send(Message::Text(
                json!([
                    "OK",
                    event.id().to_string(),
                    true,
                    format!("{}/{}", CONFIG.endpoint, event.pubkey())
                ])
                .to_string()
                .into(),
            ))
            .await
            .ok();

        let (relay_sender_tx, relay_sender_rx) = mpsc::channel::<RelaySenderCommand>(32);
        let (proxy_sender_tx, proxy_sender_rx) = mpsc::channel::<(String, Message)>(32);
        let node = Self {
            id: event.pubkey().to_string(),
            is_active: Arc::new(AtomicBool::new(true)),
            last_active: Arc::new(AtomicU64::new(unix_timestamp())),
            relay_sender: relay_sender_tx,
            proxy_sender: proxy_sender_tx,
            clients: Arc::new(DashMap::new()),
            in_progress_events: Arc::new(DashMap::new()),
        };
        node.handle_send_to_relay_messages(sender, relay_sender_rx);
        node.handle_relay_messages(receiver);
        node.handle_client_messages(proxy_sender_rx);
        node.cleanup();
        Some(node)
    }

    pub fn id(&self) -> String {
        self.id.clone()
    }

    pub fn is_active(&self) -> bool {
        self.is_active.load(Ordering::Relaxed)
    }

    pub fn seconds_since_last_active(&self) -> u64 {
        let last_active = self.last_active.load(Ordering::Relaxed);
        let now = unix_timestamp();
        if now >= last_active {
            now - last_active
        } else {
            0
        }
    }

    pub async fn close(&self) {
        if !self.is_active.load(Ordering::Relaxed) {
            return;
        }
        let _ = self.relay_sender.send(RelaySenderCommand::Close).await;
    }

    pub fn add_client(&self, ws: WebSocket) {
        let clients = Arc::clone(&self.clients);
        let client_id = loop {
            let client_id = rand::random::<u16>().to_string();
            if !clients.contains_key(&client_id) {
                break client_id;
            }
        };

        let client = Client::new(client_id.clone(), ws, self.proxy_sender.clone());
        clients.insert(client_id.clone(), Arc::new(client));
        info!(
            "[NODE:{}] + CLIENT:{} (total clients: {})",
            self.id,
            client_id,
            clients.len()
        );
    }

    fn handle_send_to_relay_messages(
        &self,
        mut sender: SplitSink<WebSocket, Message>,
        mut cmd_rx: Receiver<RelaySenderCommand>,
    ) {
        let is_active = Arc::clone(&self.is_active);
        let clients = Arc::clone(&self.clients);

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    RelaySenderCommand::Send(msg) => {
                        if sender.send(msg).await.is_err() {
                            let _ = sender.close().await;
                            for client in clients.iter() {
                                client.value().close().await;
                            }
                            is_active.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                    RelaySenderCommand::Close => {
                        let _ = sender.close().await;
                        for client in clients.iter() {
                            client.value().close().await;
                        }
                        is_active.store(false, Ordering::Relaxed);
                        break;
                    }
                }
            }
        });
    }

    fn handle_relay_messages(&self, mut receiver: SplitStream<WebSocket>) {
        let last_active = Arc::clone(&self.last_active);
        let clients = Arc::clone(&self.clients);
        let in_progress_events = Arc::clone(&self.in_progress_events);
        let relay_sender = self.relay_sender.clone();

        tokio::spawn(async move {
            while let Some(msg) = receiver.next().await {
                last_active.store(unix_timestamp(), Ordering::Relaxed);
                match msg {
                    Ok(Message::Text(text)) => {
                        let parsed: serde_json::Value = match serde_json::from_str(&text) {
                            Ok(parsed) => parsed,
                            Err(_) => continue,
                        };
                        if parsed.is_array() {
                            let arr = match parsed.as_array() {
                                Some(arr) => arr,
                                None => continue,
                            };
                            if arr[0] == "EVENT" || arr[0] == "EOSE" || arr[0] == "CLOSED" {
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
                                    new_arr.push(arr[0].clone());
                                    // Restore the original subscription ID
                                    new_arr.push(json!(raw_sub_id));

                                    // Add all remaining elements from the original array
                                    for i in 2..arr.len() {
                                        new_arr.push(arr[i].clone());
                                    }

                                    client
                                        .send(Message::Text(json!(new_arr).to_string().into()))
                                        .await;
                                }
                            } else if arr[0] == "OK" {
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
                    }
                    Ok(Message::Ping(_)) => {
                        let _ = relay_sender
                            .send(RelaySenderCommand::Send(Message::Pong(Vec::new().into())))
                            .await;
                    }
                    Ok(Message::Close(_)) => {
                        let _ = relay_sender.send(RelaySenderCommand::Close).await;
                        break;
                    }
                    Err(_) => {
                        let _ = relay_sender.send(RelaySenderCommand::Close).await;
                        break;
                    }
                    _ => {}
                }
            }
        });
    }

    fn handle_client_messages(&self, mut receiver: Receiver<(String, Message)>) {
        let in_progress_events = Arc::clone(&self.in_progress_events);
        let relay_sender = self.relay_sender.clone();

        tokio::spawn(async move {
            while let Some(msg) = receiver.recv().await {
                match msg {
                    (client_id, Message::Text(text)) => {
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
                        if arr[0] == "REQ" || arr[0] == "CLOSE" {
                            let sub_id = match arr[1].as_str() {
                                Some(sub_id) => sub_id,
                                None => continue,
                            };

                            let mut new_arr = Vec::new();
                            new_arr.push(json!("REQ"));
                            // Add client ID to the subscription ID
                            new_arr.push(json!(format!("{}:{}", client_id, sub_id)));

                            // Add all remaining elements from the original array
                            for i in 2..arr.len() {
                                new_arr.push(arr[i].clone());
                            }

                            let _ = relay_sender
                                .send(RelaySenderCommand::Send(Message::Text(
                                    json!(new_arr).to_string().into(),
                                )))
                                .await;
                        } else if arr[0] == "EVENT" {
                            let event = match NostrEvent::from_value(&arr[1]) {
                                Ok(event) => event,
                                Err(_) => continue,
                            };
                            in_progress_events
                                .entry(event.id().to_string())
                                .or_insert_with(Vec::new)
                                .push(client_id);

                            let _ = relay_sender
                                .send(RelaySenderCommand::Send(Message::Text(text)))
                                .await;
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    fn cleanup(&self) {
        let node_id = self.id.clone();
        let clients = Arc::clone(&self.clients);

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;

                let to_remove = clients
                    .iter()
                    .filter_map(|entry| {
                        let client = entry.value();
                        if !client.is_active() || client.seconds_since_last_active() > 60 {
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

fn check_auth_message(message: &str, challenge: &str) -> Option<NostrEvent> {
    let parsed: serde_json::Value = match serde_json::from_str(message) {
        Ok(parsed) => parsed,
        Err(_) => return None,
    };
    let arr = match parsed.as_array() {
        Some(arr) => arr,
        None => return None,
    };

    if (arr.len() != 2) || (arr[0] != "AUTH") {
        return None;
    }

    let event = match NostrEvent::from_value(&arr[1]) {
        Ok(event) => event,
        Err(_) => return None,
    };

    if !event.verify() {
        return None;
    }

    if event.kind() != 22242 {
        return None;
    }

    let now = unix_timestamp();
    if now.abs_diff(event.created_at()) > 60 {
        return None;
    }

    let mut match_challenge = false;
    let mut match_relay = false;

    for tag in event.tags() {
        if tag.len() >= 2 && tag[0] == "challenge" && tag[1] == challenge {
            match_challenge = true;
        }

        if tag.len() >= 2 && tag[0] == "relay" && !match_relay {
            if let Some(domain) = extract_endpoint_from_url(&tag[1]) {
                match_relay = domain == CONFIG.endpoint;
            }
        }
    }

    if match_challenge && match_relay {
        Some(event)
    } else {
        None
    }
}
