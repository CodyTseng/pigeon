mod tests {
    use futures_util::{SinkExt, StreamExt};
    use nostr::prelude::*;
    use pigeon_nest::create_app;
    use serde_json::json;
    use std::{borrow::Cow, net::SocketAddr};
    use tokio::{
        net::TcpListener,
        time::{Duration, sleep},
    };
    use tokio_tungstenite::connect_async;
    use tungstenite::Message;

    #[tokio::test]
    async fn test_relay_client_interaction() {
        let app = create_app();
        let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
        let listener = TcpListener::bind(addr)
            .await
            .expect("Failed to bind to address");

        // Start the server in a background task
        tokio::spawn(async move {
            axum::serve(listener, app.into_make_service())
                .await
                .expect("Server error");
        });

        // Wait briefly for the server to start
        sleep(Duration::from_millis(100)).await;

        // Relay registers via /register
        let secret_key = Keys::generate();
        let register_url = "ws://localhost:3000/register";
        let (mut relay_ws, _) = connect_async(register_url)
            .await
            .expect("Failed to connect relay");

        let auth_request_msg = relay_ws
            .next()
            .await
            .expect("No message received from relay registration")
            .expect("Error reading message");
        let auth_event = match auth_request_msg {
            Message::Text(msg) => {
                let auth_request: Vec<String> =
                    serde_json::from_str(&msg).expect("Failed to parse auth response");
                assert_eq!(auth_request.len(), 2);
                assert_eq!(auth_request[0], "AUTH");

                let challenge = auth_request[1].clone();
                assert!(!challenge.is_empty());

                let relay_url = RelayUrl::parse(&register_url).unwrap();
                let auth_event: Event = EventBuilder::auth(challenge, relay_url)
                    .sign_with_keys(&secret_key)
                    .unwrap();
                let json = ClientMessage::Auth(Cow::Owned(auth_event.clone())).as_json();
                relay_ws
                    .send(Message::Text(json.into()))
                    .await
                    .expect("Failed to send auth event");
                auth_event
            }
            _ => panic!("Unexpected message type"),
        };

        let auth_response_msg = relay_ws
            .next()
            .await
            .expect("No message received from relay after auth")
            .expect("Error reading message");

        let node_id = match auth_response_msg {
            Message::Text(msg) => {
                let auth_response: serde_json::Value =
                    serde_json::from_str(&msg).expect("Failed to parse auth response");
                assert!(auth_response.is_array());

                let auth_response = auth_response.as_array().unwrap();
                assert_eq!(auth_response.len(), 4);
                assert_eq!(auth_response[0], "OK");
                assert_eq!(auth_response[1], auth_event.id.to_hex());
                assert!(auth_response[2].as_bool().unwrap());

                secret_key.public_key().to_hex()
            }
            _ => panic!("Unexpected message type"),
        };

        let reqwest_client = reqwest::Client::new();
        let index_response = reqwest_client
            .get(format!("http://localhost:3000/"))
            .send()
            .await
            .expect("Failed to get index endpoint");
        assert_eq!(index_response.status(), 200);
        assert_eq!(
            index_response.text().await.unwrap(),
            "Proxying for 1 relays"
        );

        // Client connects to the relay using the node_id
        let client_url = format!("ws://localhost:3000/{}", node_id);
        let (mut client_ws, _) = connect_async(&client_url)
            .await
            .expect("Failed to connect client");

        // Client sends a subscription request
        client_ws
            .send(Message::Text(
                json!(["REQ", "test", { "kinds": [1] }]).to_string().into(),
            ))
            .await
            .expect("Client failed to send message");

        // Relay receives the subscription request
        let client_msg = relay_ws
            .next()
            .await
            .expect("Relay did not receive client message")
            .expect("Error receiving client message");
        let sub_id = match client_msg {
            Message::Text(msg) => {
                let parsed: serde_json::Value =
                    serde_json::from_str(&msg).expect("Failed to parse client message");
                assert!(parsed.is_array());
                let arr = parsed.as_array().unwrap();
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], "REQ");
                arr[1].as_str().unwrap().to_string()
            }
            _ => panic!("Unexpected message type"),
        };

        // Relay sends an EOSE message back to the client
        relay_ws
            .send(Message::Text(json!(["EOSE", sub_id]).to_string().into()))
            .await
            .expect("Relay failed to send message");

        // Client should receive the EOSE message
        let relay_msg = client_ws
            .next()
            .await
            .expect("Client did not receive relay message")
            .expect("Error receiving relay message");
        assert_eq!(
            relay_msg,
            Message::Text(json!(["EOSE", "test"]).to_string().into())
        );
    }
}
