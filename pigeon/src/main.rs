use anyhow::anyhow;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use nostr::prelude::*;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[clap(author, version)]
struct Args {
    /// URL of the relay
    #[clap(short = 'r', long)]
    relay: Option<String>,

    /// URL of the proxy service
    #[clap(short = 'p', long)]
    proxy: Option<String>,

    /// Optional: hex-or-bech32-secret-key
    #[clap(short = 's', long)]
    secret_key: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Parse command line arguments
    let args = Args::parse();

    let secret_key = if let Some(secret_key) = args.secret_key {
        Keys::parse(&secret_key).unwrap()
    } else {
        Keys::generate()
    };

    let proxy_url = Url::parse(
        &args
            .proxy
            .clone()
            .unwrap_or("wss://proxy.nostr-relay.app/".to_string()),
    )
    .unwrap()
    .join("register")
    .unwrap();
    let proxy = proxy_url.to_string();
    info!("Connecting to proxy: {}", proxy_url.to_string());

    // Connect to both services
    let (proxy_ws, _) = connect_async(proxy_url.to_string()).await.unwrap();
    let (proxy_write, mut proxy_read) = proxy_ws.split();
    let shared_proxy_writer = Arc::new(Mutex::new(proxy_write));
    let mut public_relay_url = "".to_string();
    while let Some(msg) = proxy_read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                let parsed: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(parsed) => parsed,
                    Err(_) => {
                        return Err(anyhow!("Received unexpected message from proxy: {}", text));
                    }
                };
                if !parsed.is_array() {
                    return Err(anyhow!("Received unexpected message from proxy: {}", text));
                }
                let arr = match parsed.as_array() {
                    Some(arr) => arr,
                    None => {
                        return Err(anyhow!("Received unexpected message from proxy: {}", text));
                    }
                };
                if arr[0] == "AUTH" {
                    let challenge = arr[1].as_str().unwrap_or("");
                    let relay_url = RelayUrl::parse(&proxy).unwrap();
                    let auth_event: Event = EventBuilder::auth(challenge, relay_url)
                        .sign_with_keys(&secret_key)
                        .unwrap();
                    let json = ClientMessage::Auth(Box::new(auth_event)).as_json();
                    shared_proxy_writer
                        .lock()
                        .await
                        .send(Message::Text(json.to_string().into()))
                        .await
                        .unwrap();
                } else if arr[0] == "OK" {
                    info!("Connected to proxy");
                    public_relay_url = arr[3].as_str().unwrap_or("").to_string();
                    break;
                }
            }
            _ => {
                return Err(anyhow!("Received unexpected message from proxy: {:?}", msg));
            }
        }
    }

    let ping_writer = shared_proxy_writer.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = ping_writer
                .lock()
                .await
                .send(Message::Ping(Vec::new().into()))
                .await
            {
                error!("Error sending ping to proxy: {}", e);
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(180)).await;
        }
    });

    let relay_url = Url::parse(
        &args
            .relay
            .clone()
            .unwrap_or("ws://localhost:4869/".to_string()),
    )
    .unwrap();
    info!("Connecting to relay: {}", &relay_url);
    let (relay_ws, _) = connect_async(&relay_url.to_string()).await.unwrap();
    info!("Connected to relay");
    let (mut relay_write, mut relay_read) = relay_ws.split();

    let proxy_to_relay = tokio::spawn(async move {
        while let Some(msg) = proxy_read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Err(e) = relay_write.send(Message::Text(text)).await {
                        error!("Error forwarding message to proxy: {}", e);
                        return;
                    }
                }
                Err(e) => {
                    error!("Error receiving message from proxy: {}", e);
                    return;
                }
                _ => {}
            }
        }
    });

    let relay_to_proxy = tokio::spawn(async move {
        while let Some(msg) = relay_read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Err(e) = shared_proxy_writer
                        .lock()
                        .await
                        .send(Message::Text(text))
                        .await
                    {
                        error!("Error forwarding message to proxy: {}", e);
                        return;
                    }
                }
                Err(e) => {
                    error!("Error receiving message from relay: {}", e);
                    return;
                }
                _ => {}
            }
        }
    });

    info!(
        "Forwarding: {} <-> {}",
        public_relay_url,
        relay_url.to_string()
    );

    // Wait for both tasks to complete
    tokio::select! {
        _ = proxy_to_relay => println!("proxy_to_relay task completed"),
        _ = relay_to_proxy => println!("relay_to_proxy task completed"),
    }

    Ok(())
}
