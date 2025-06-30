use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use jsonwebtoken::{encode, EncodingKey, Header};
use webrtc_signaling::messages::{ClientMessage, ServerMessage};
use webrtc_signaling::server::start_server;

// Helper function to create a test JWT token
fn create_test_token(secret: &str, user_id: u32, username: &str) -> String {
    use webrtc_signaling::auth::Claims;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;

    let claims = Claims {
        sub: user_id,
        username: username.to_string(),
        iat: now,
        exp: now + 3600, // Valid for 1 hour
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_ref()),
    )
    .unwrap()
}

// Helper function to find an available port
async fn find_available_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[tokio::test]
async fn test_websocket_authentication_success() {
    let port = find_available_port().await;
    let jwt_secret = "test_secret_key";
    let token = create_test_token(jwt_secret, 123, "testuser");

    // Start server in background
    let server_handle = tokio::spawn(async move {
        start_server("127.0.0.1".to_string(), port, jwt_secret.to_string()).await
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect to WebSocket
    let ws_url = format!("ws://127.0.0.1:{}", port);
    let (ws_stream, _) = connect_async(&ws_url).await.expect("Failed to connect");
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Send authentication message
    let auth_msg = ClientMessage::Auth { token };
    let auth_json = serde_json::to_string(&auth_msg).unwrap();
    ws_sender.send(Message::Text(auth_json)).await.unwrap();

    // Receive authentication response
    if let Some(Ok(Message::Text(response))) = ws_receiver.next().await {
        let server_msg: ServerMessage = serde_json::from_str(&response).unwrap();
        match server_msg {
            ServerMessage::Authenticated { user_id, username } => {
                assert_eq!(user_id, 123);
                assert_eq!(username, "testuser");
            }
            _ => panic!("Expected authenticated message, got: {:?}", server_msg),
        }
    } else {
        panic!("No response received");
    }

    // Clean up
    server_handle.abort();
}

#[tokio::test]
async fn test_websocket_authentication_failure() {
    let port = find_available_port().await;
    let jwt_secret = "test_secret_key";
    let invalid_token = "invalid.jwt.token";

    // Start server in background
    let server_handle = tokio::spawn(async move {
        start_server("127.0.0.1".to_string(), port, jwt_secret.to_string()).await
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect to WebSocket
    let ws_url = format!("ws://127.0.0.1:{}", port);
    let (ws_stream, _) = connect_async(&ws_url).await.expect("Failed to connect");
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Send invalid authentication message
    let auth_msg = ClientMessage::Auth {
        token: invalid_token.to_string(),
    };
    let auth_json = serde_json::to_string(&auth_msg).unwrap();
    ws_sender.send(Message::Text(auth_json)).await.unwrap();

    // Receive error response
    if let Some(Ok(Message::Text(response))) = ws_receiver.next().await {
        let server_msg: ServerMessage = serde_json::from_str(&response).unwrap();
        match server_msg {
            ServerMessage::Error { message, .. } => {
                assert!(message.contains("Authentication failed"));
            }
            _ => panic!("Expected error message, got: {:?}", server_msg),
        }
    } else {
        panic!("No response received");
    }

    // Clean up
    server_handle.abort();
}

#[tokio::test]
async fn test_room_join_and_leave_flow() {
    let port = find_available_port().await;
    let jwt_secret = "test_secret_key";
    let token = create_test_token(jwt_secret, 123, "testuser");

    // Start server in background
    let server_handle = tokio::spawn(async move {
        start_server("127.0.0.1".to_string(), port, jwt_secret.to_string()).await
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect to WebSocket
    let ws_url = format!("ws://127.0.0.1:{}", port);
    let (ws_stream, _) = connect_async(&ws_url).await.expect("Failed to connect");
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Authenticate
    let auth_msg = ClientMessage::Auth { token };
    let auth_json = serde_json::to_string(&auth_msg).unwrap();
    ws_sender.send(Message::Text(auth_json)).await.unwrap();

    // Consume authentication response
    let _auth_response = ws_receiver.next().await;

    // Join a room
    let join_msg = ClientMessage::JoinRoom {
        room_name: "test_room".to_string(),
        password: None,
    };
    let join_json = serde_json::to_string(&join_msg).unwrap();
    ws_sender.send(Message::Text(join_json)).await.unwrap();

    // Receive room joined response
    if let Some(Ok(Message::Text(response))) = ws_receiver.next().await {
        let server_msg: ServerMessage = serde_json::from_str(&response).unwrap();
        match server_msg {
            ServerMessage::RoomJoined {
                room_name,
                user_id,
                participants,
            } => {
                assert_eq!(room_name, "test_room");
                assert_eq!(user_id, 123);
                assert!(participants.is_empty()); // No other participants
            }
            _ => panic!("Expected room joined message, got: {:?}", server_msg),
        }
    }

    // Leave the room
    let leave_msg = ClientMessage::LeaveRoom {
        room_name: "test_room".to_string(),
    };
    let leave_json = serde_json::to_string(&leave_msg).unwrap();
    ws_sender.send(Message::Text(leave_json)).await.unwrap();

    // Receive room left response
    if let Some(Ok(Message::Text(response))) = ws_receiver.next().await {
        let server_msg: ServerMessage = serde_json::from_str(&response).unwrap();
        match server_msg {
            ServerMessage::RoomLeft { room_name, user_id } => {
                assert_eq!(room_name, "test_room");
                assert_eq!(user_id, 123);
            }
            _ => panic!("Expected room left message, got: {:?}", server_msg),
        }
    }

    // Clean up
    server_handle.abort();
}

#[tokio::test]
async fn test_multiple_users_in_room() {
    let port = find_available_port().await;
    let jwt_secret = "test_secret_key";
    let token1 = create_test_token(jwt_secret, 123, "user1");
    let token2 = create_test_token(jwt_secret, 456, "user2");

    // Start server in background
    let server_handle = tokio::spawn(async move {
        start_server("127.0.0.1".to_string(), port, jwt_secret.to_string()).await
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://127.0.0.1:{}", port);

    // Connect first user
    let (ws_stream1, _) = connect_async(&ws_url)
        .await
        .expect("Failed to connect user1");
    let (mut ws_sender1, mut ws_receiver1) = ws_stream1.split();

    // Connect second user
    let (ws_stream2, _) = connect_async(&ws_url)
        .await
        .expect("Failed to connect user2");
    let (mut ws_sender2, mut ws_receiver2) = ws_stream2.split();

    // Authenticate both users
    let auth_msg1 = ClientMessage::Auth { token: token1 };
    let auth_json1 = serde_json::to_string(&auth_msg1).unwrap();
    ws_sender1.send(Message::Text(auth_json1)).await.unwrap();
    let _auth_response1 = ws_receiver1.next().await;

    let auth_msg2 = ClientMessage::Auth { token: token2 };
    let auth_json2 = serde_json::to_string(&auth_msg2).unwrap();
    ws_sender2.send(Message::Text(auth_json2)).await.unwrap();
    let _auth_response2 = ws_receiver2.next().await;

    // User1 joins room
    let join_msg1 = ClientMessage::JoinRoom {
        room_name: "test_room".to_string(),
        password: None,
    };
    let join_json1 = serde_json::to_string(&join_msg1).unwrap();
    ws_sender1.send(Message::Text(join_json1)).await.unwrap();
    let _join_response1 = ws_receiver1.next().await; // Room joined for user1

    // User2 joins the same room
    let join_msg2 = ClientMessage::JoinRoom {
        room_name: "test_room".to_string(),
        password: None,
    };
    let join_json2 = serde_json::to_string(&join_msg2).unwrap();
    ws_sender2.send(Message::Text(join_json2)).await.unwrap();

    // User2 should receive room joined with user1 as existing participant
    if let Some(Ok(Message::Text(response))) = ws_receiver2.next().await {
        let server_msg: ServerMessage = serde_json::from_str(&response).unwrap();
        match server_msg {
            ServerMessage::RoomJoined {
                room_name,
                user_id,
                participants,
            } => {
                assert_eq!(room_name, "test_room");
                assert_eq!(user_id, 456);
                assert_eq!(participants.len(), 1);
                assert_eq!(participants[0].user_id, 123);
            }
            _ => panic!("Expected room joined message, got: {:?}", server_msg),
        }
    }

    // User1 should receive user joined notification
    if let Some(Ok(Message::Text(response))) = ws_receiver1.next().await {
        let server_msg: ServerMessage = serde_json::from_str(&response).unwrap();
        match server_msg {
            ServerMessage::UserJoined { room_name, user } => {
                assert_eq!(room_name, "test_room");
                assert_eq!(user.user_id, 456);
                assert_eq!(user.username, "user2");
            }
            _ => panic!("Expected user joined message, got: {:?}", server_msg),
        }
    }

    // Clean up
    server_handle.abort();
}

#[tokio::test]
async fn test_webrtc_signaling_flow() {
    let port = find_available_port().await;
    let jwt_secret = "test_secret_key";
    let token1 = create_test_token(jwt_secret, 123, "user1");
    let token2 = create_test_token(jwt_secret, 456, "user2");

    // Start server in background
    let server_handle = tokio::spawn(async move {
        start_server("127.0.0.1".to_string(), port, jwt_secret.to_string()).await
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://127.0.0.1:{}", port);

    // Connect and authenticate both users, join same room (similar to previous test)
    let (ws_stream1, _) = connect_async(&ws_url)
        .await
        .expect("Failed to connect user1");
    let (mut ws_sender1, mut ws_receiver1) = ws_stream1.split();

    let (ws_stream2, _) = connect_async(&ws_url)
        .await
        .expect("Failed to connect user2");
    let (mut ws_sender2, mut ws_receiver2) = ws_stream2.split();

    // Quick setup (authenticate and join room)
    let auth_msg1 = ClientMessage::Auth { token: token1 };
    ws_sender1
        .send(Message::Text(serde_json::to_string(&auth_msg1).unwrap()))
        .await
        .unwrap();
    let _auth_response1 = ws_receiver1.next().await;

    let auth_msg2 = ClientMessage::Auth { token: token2 };
    ws_sender2
        .send(Message::Text(serde_json::to_string(&auth_msg2).unwrap()))
        .await
        .unwrap();
    let _auth_response2 = ws_receiver2.next().await;

    let join_msg1 = ClientMessage::JoinRoom {
        room_name: "test_room".to_string(),
        password: None,
    };
    ws_sender1
        .send(Message::Text(serde_json::to_string(&join_msg1).unwrap()))
        .await
        .unwrap();
    let _join_response1 = ws_receiver1.next().await;

    let join_msg2 = ClientMessage::JoinRoom {
        room_name: "test_room".to_string(),
        password: None,
    };
    ws_sender2
        .send(Message::Text(serde_json::to_string(&join_msg2).unwrap()))
        .await
        .unwrap();
    let _join_response2 = ws_receiver2.next().await;
    let _user_joined = ws_receiver1.next().await; // User1 receives user2 joined notification

    // Test WebRTC offer
    let offer_msg = ClientMessage::Offer {
        room_name: "test_room".to_string(),
        sdp: "test_offer_sdp".to_string(),
        target_user_id: Some(456),
    };
    ws_sender1
        .send(Message::Text(serde_json::to_string(&offer_msg).unwrap()))
        .await
        .unwrap();

    // User2 should receive the offer
    if let Some(Ok(Message::Text(response))) = ws_receiver2.next().await {
        let server_msg: ServerMessage = serde_json::from_str(&response).unwrap();
        match server_msg {
            ServerMessage::Offer {
                room_name,
                from_user_id,
                sdp,
            } => {
                assert_eq!(room_name, "test_room");
                assert_eq!(from_user_id, 123);
                assert_eq!(sdp, "test_offer_sdp");
            }
            _ => panic!("Expected offer message, got: {:?}", server_msg),
        }
    }

    // Test WebRTC answer
    let answer_msg = ClientMessage::Answer {
        room_name: "test_room".to_string(),
        sdp: "test_answer_sdp".to_string(),
        target_user_id: 123,
    };
    ws_sender2
        .send(Message::Text(serde_json::to_string(&answer_msg).unwrap()))
        .await
        .unwrap();

    // User1 should receive the answer
    if let Some(Ok(Message::Text(response))) = ws_receiver1.next().await {
        let server_msg: ServerMessage = serde_json::from_str(&response).unwrap();
        match server_msg {
            ServerMessage::Answer {
                room_name,
                from_user_id,
                sdp,
            } => {
                assert_eq!(room_name, "test_room");
                assert_eq!(from_user_id, 456);
                assert_eq!(sdp, "test_answer_sdp");
            }
            _ => panic!("Expected answer message, got: {:?}", server_msg),
        }
    }

    // Test ICE candidate exchange
    let ice_msg = ClientMessage::IceCandidate {
        room_name: "test_room".to_string(),
        candidate: "test_ice_candidate".to_string(),
        sdp_mid: Some("0".to_string()),
        sdp_mline_index: Some(0),
        target_user_id: Some(456),
    };
    ws_sender1
        .send(Message::Text(serde_json::to_string(&ice_msg).unwrap()))
        .await
        .unwrap();

    // User2 should receive the ICE candidate
    if let Some(Ok(Message::Text(response))) = ws_receiver2.next().await {
        let server_msg: ServerMessage = serde_json::from_str(&response).unwrap();
        match server_msg {
            ServerMessage::IceCandidate {
                room_name,
                from_user_id,
                candidate,
                sdp_mid,
                sdp_mline_index,
            } => {
                assert_eq!(room_name, "test_room");
                assert_eq!(from_user_id, 123);
                assert_eq!(candidate, "test_ice_candidate");
                assert_eq!(sdp_mid, Some("0".to_string()));
                assert_eq!(sdp_mline_index, Some(0));
            }
            _ => panic!("Expected ICE candidate message, got: {:?}", server_msg),
        }
    }

    // Clean up
    server_handle.abort();
}
