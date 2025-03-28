use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use futures::SinkExt;
use futures::stream::{SplitSink, SplitStream, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, mpsc::Sender};
use tokio::time::timeout;
use warp::filters::ws::{Message, WebSocket};

use crate::config::CONFIG;
use crate::nostr_event::NostrEvent;
use crate::utils::{extract_endpoint_from_url, unix_timestamp};

enum SenderCommand {
    Send(Message),
    Close,
}

pub struct Relay {
    id: String,
    is_active: Arc<AtomicBool>,
    last_active: Arc<AtomicU64>,
    sender: Sender<SenderCommand>,
    node_sender: Sender<Message>,
    relay_info: String,
}

impl Relay {
    pub async fn new(ws: WebSocket, node_sender: Sender<Message>) -> Option<Self> {
        let (mut sender, mut receiver) = ws.split();

        let challenge = format!("{:x}", rand::random::<u128>());
        let auth_request = json!(["AUTH", challenge]);
        if sender
            .send(Message::text(auth_request.to_string()))
            .await
            .is_err()
        {
            return None;
        }
        let auth_message = match timeout(std::time::Duration::from_secs(5), receiver.next()).await {
            Ok(Some(Ok(msg))) => msg,
            _ => return None,
        };

        let event = if auth_message.is_text() {
            let message = match auth_message.to_str() {
                Ok(message) => message,
                Err(_) => return None,
            };
            check_auth_message(message, &challenge)?
        } else {
            return None;
        };

        sender
            .send(Message::text(
                json!([
                    "OK",
                    event.id().to_string(),
                    true,
                    format!("{}/{}", CONFIG.endpoint, event.pubkey())
                ])
                .to_string(),
            ))
            .await
            .ok();

        let (tx, rx) = mpsc::channel(32);
        let relay = Self {
            id: event.pubkey().to_string(),
            is_active: Arc::new(AtomicBool::new(true)),
            last_active: Arc::new(AtomicU64::new(unix_timestamp())),
            sender: tx,
            node_sender,
            relay_info: event.content().to_string(),
        };
        relay.handle_send_to_relay_message(sender, rx);
        relay.handle_relay_messages(receiver);
        Some(relay)
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
        if now > last_active {
            now - last_active
        } else {
            0
        }
    }

    pub fn relay_info(&self) -> &str {
        &self.relay_info
    }

    pub async fn close(&self) {
        if !self.is_active.load(Ordering::Relaxed) {
            return;
        }
        let _ = self.sender.send(SenderCommand::Close).await;
    }

    pub async fn send(&self, msg: Message) {
        if !self.is_active.load(Ordering::Relaxed) {
            return;
        }
        let _ = self.sender.send(SenderCommand::Send(msg)).await;
    }

    fn handle_send_to_relay_message(
        &self,
        mut sender: SplitSink<WebSocket, Message>,
        mut rx: mpsc::Receiver<SenderCommand>,
    ) {
        let is_active = Arc::clone(&self.is_active);

        tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                let should_close = match cmd {
                    SenderCommand::Send(msg) => sender.send(msg).await.is_err(),
                    SenderCommand::Close => true,
                };

                if should_close {
                    let _ = sender.close().await;
                    is_active.store(false, Ordering::Relaxed);
                    break;
                }
            }
        });
    }

    fn handle_relay_messages(&self, mut receiver: SplitStream<WebSocket>) {
        let last_active = Arc::clone(&self.last_active);
        let sender = self.sender.clone();
        let node_sender = self.node_sender.clone();

        tokio::spawn(async move {
            while let Some(msg_result) = receiver.next().await {
                last_active.store(unix_timestamp(), Ordering::Relaxed);
                if let Ok(msg) = msg_result {
                    if msg.is_text() {
                        let _ = node_sender.send(msg).await;
                    } else if msg.is_close() {
                        let _ = sender.send(SenderCommand::Close).await;
                        break;
                    }
                } else {
                    let _ = sender.send(SenderCommand::Close).await;
                    break;
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
    if !parsed.is_array() {
        return None;
    }
    let arr = match parsed.as_array() {
        Some(arr) => arr,
        None => return None,
    };
    if arr.len() < 2 {
        return None;
    }

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
        if tag.len() < 2 {
            continue;
        }

        let (tag_name, tag_value) = match (tag.get(0), tag.get(1)) {
            (Some(name), Some(value)) => (name, value),
            _ => continue,
        };

        if tag.len() >= 2 && tag_name == "challenge" && tag_value == challenge {
            match_challenge = true;
        }

        if tag.len() >= 2 && tag_name == "relay" && !match_relay {
            if let Some(domain) = extract_endpoint_from_url(tag_value) {
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
