use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use axum::extract::ws::{Message, WebSocket};
use futures::SinkExt;
use futures::stream::{SplitSink, SplitStream, StreamExt};
use tokio::sync::{mpsc, mpsc::Sender};

use crate::utils::unix_timestamp;

enum SenderCommand {
    Send(Message),
    Close,
}

pub struct Client {
    is_active: Arc<AtomicBool>,
    last_active: Arc<AtomicU64>,
    sender: Sender<SenderCommand>,
    node_sender: Sender<(String, Message)>,
}

impl Client {
    pub fn new(id: String, ws: WebSocket, node_sender: Sender<(String, Message)>) -> Self {
        let (sender, receiver) = ws.split();
        let (tx, rx) = mpsc::channel(32);

        let client = Self {
            is_active: Arc::new(AtomicBool::new(true)),
            last_active: Arc::new(AtomicU64::new(unix_timestamp())),
            sender: tx,
            node_sender,
        };
        client.handle_send_to_client_messages(sender, rx);
        client.handle_client_messages(id.clone(), receiver);
        client
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

    fn handle_send_to_client_messages(
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

    fn handle_client_messages(&self, id: String, mut receiver: SplitStream<WebSocket>) {
        let last_active = Arc::clone(&self.last_active);
        let sender = self.sender.clone();
        let node_sender = self.node_sender.clone();

        tokio::spawn(async move {
            while let Some(msg) = receiver.next().await {
                last_active.store(unix_timestamp(), Ordering::Relaxed);
                match msg {
                    Ok(Message::Text(text)) => {
                        let _ = node_sender.send((id.clone(), Message::Text(text))).await;
                    }
                    Ok(Message::Ping(_)) => {
                        let _ = sender
                            .send(SenderCommand::Send(Message::Pong(Vec::new().into())))
                            .await;
                    }
                    Ok(Message::Close(_)) => {
                        let _ = sender.send(SenderCommand::Close).await;
                        break;
                    }
                    Err(_) => {
                        let _ = sender.send(SenderCommand::Close).await;
                        break;
                    }
                    _ => {}
                }
            }
        });
    }
}
