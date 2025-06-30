use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use uuid::Uuid;
// Removed url dependency
use tracing::{debug, error, info};

use crate::auth::JwtValidator;
use crate::messages::{ClientMessage, Publisher, ServerMessage};
use crate::rheomesh_sfu::RheomeshSfu;
use crate::room::LocalRoomManager;
use crate::room::Room;
use crate::room::{RoomManager, RoomParticipant, Rooms};
use rheomesh::transport::Transport;
use std::collections::HashMap;
use tokio::sync::RwLock;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

pub async fn start_server(
    host: String,
    port: u16,
    jwt_secret: String,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let room_manager = RoomManager::new();
    start_server_with_room_manager_and_sfu(host, port, jwt_secret, room_manager, None).await
}

pub async fn start_server_with_room_manager(
    host: String,
    port: u16,
    jwt_secret: String,
    room_manager: RoomManager,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    start_server_with_room_manager_and_sfu(host, port, jwt_secret, room_manager, None).await
}

pub async fn start_server_with_room_manager_and_sfu(
    host: String,
    port: u16,
    jwt_secret: String,
    room_manager: RoomManager,
    sfu: Option<Arc<RheomeshSfu>>,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;

    info!("WebSocket server listening on: {}", addr);

    let jwt_validator = Arc::new(JwtValidator::new(&jwt_secret));
    let room_manager = Arc::new(room_manager);

    while let Ok((stream, peer_addr)) = listener.accept().await {
        info!("New connection from: {}", peer_addr);

        let jwt_validator = jwt_validator.clone();
        let room_manager = room_manager.clone();
        let sfu = sfu.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, jwt_validator, room_manager, sfu).await {
                error!("Connection error: {}", e);
            }
        });
    }

    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    jwt_validator: Arc<JwtValidator>,
    room_manager: Arc<RoomManager>,
    sfu: Option<Arc<RheomeshSfu>>,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection_id = Uuid::new_v4();

    let ws_stream = accept_async(stream).await?;
    debug!("WebSocket connection established: {}", connection_id);

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    // Handle outgoing messages
    let outgoing_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if let Err(e) = ws_sender.send(message).await {
                error!("Failed to send WebSocket message: {}", e);
                break;
            }
        }
    });

    // Wait for authentication message (first message should be auth)
    let user = match authenticate_connection(&mut ws_receiver, &jwt_validator).await {
        Ok(user) => user,
        Err(e) => {
            error!("Authentication failed: {}", e);
            let error_msg = ServerMessage::error(format!("Authentication failed: {}", e));
            let _ = send_message(&tx, error_msg);
            return Ok(());
        }
    };

    println!("DEBUG: Authenticated user: {}", user.username);

    info!(
        "User {} ({}) authenticated successfully",
        user.user_id, user.username
    );

    // Send authentication confirmation
    let auth_msg = ServerMessage::Authenticated {
        user_id: user.user_id,
        username: user.username.clone(),
    };
    let _ = send_message(&tx, auth_msg);

    // Handle incoming messages
    let user_id = user.user_id;
    let incoming_task = tokio::spawn(async move {
        while let Some(msg_result) = ws_receiver.next().await {
            match msg_result {
                Ok(Message::Text(text)) => {
                    if let Err(e) =
                        handle_client_message(&text, &user, connection_id, &room_manager, &sfu, &tx)
                            .await
                    {
                        error!("Error handling message: {}", e);
                        let error_msg =
                            ServerMessage::error(format!("Message handling error: {}", e));
                        let _ = send_message(&tx, error_msg);
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("User {} closed connection", user.user_id);
                    break;
                }
                Ok(_) => {
                    // Ignore other message types
                }
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                }
            }
        }

        // Clean up user from all rooms when connection closes
        room_manager
            .remove_user_from_all_rooms(user.user_id, connection_id)
            .await;
        info!("Cleaned up user {} from all rooms", user.user_id);
    });

    // Wait for either task to complete
    tokio::select! {
        _ = outgoing_task => {
            debug!("Outgoing task completed for user {}", user_id);
        }
        _ = incoming_task => {
            debug!("Incoming task completed for user {}", user_id);
        }
    }

    Ok(())
}

async fn authenticate_connection(
    ws_receiver: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<TcpStream>,
    >,
    jwt_validator: &JwtValidator,
) -> Result<crate::auth::AuthenticatedUser, String> {
    debug!("Waiting for authentication message...");
    println!("DEBUG: authenticate_connection called");

    // Wait for the first message which should contain the JWT token
    if let Some(msg_result) = ws_receiver.next().await {
        match msg_result {
            Ok(Message::Text(text)) => {
                debug!("Received authentication message: {}", text);
                println!("DEBUG: Received authentication message: {}", text);

                // Try to parse as ClientMessage::Auth
                debug!("Attempting to parse as ClientMessage::Auth...");
                println!("DEBUG: Attempting to parse as ClientMessage::Auth...");
                match serde_json::from_str::<ClientMessage>(&text) {
                    Ok(client_message) => {
                        debug!("Successfully parsed as ClientMessage: {:?}", client_message);
                        println!(
                            "DEBUG: Successfully parsed as ClientMessage: {:?}",
                            client_message
                        );
                        match client_message {
                            ClientMessage::Auth { token } => {
                                debug!("Extracted token from Auth message: {}", token);
                                println!("DEBUG: Extracted token from Auth message: {}", token);
                                return jwt_validator.validate_token(&token);
                            }
                            _ => {
                                debug!("Parsed as non-Auth message type");
                                println!("DEBUG: Parsed as non-Auth message type");
                                return Err(
                                    "Expected Auth message, got other message type".to_string()
                                );
                            }
                        }
                    }
                    Err(e) => {
                        debug!("Failed to parse as ClientMessage: {}", e);
                        println!("DEBUG: Failed to parse as ClientMessage: {}", e);
                    }
                }

                // Fallback: try to parse as generic JSON with token field
                debug!("Attempting fallback: parsing as generic JSON...");
                if let Ok(auth_msg) = serde_json::from_str::<serde_json::Value>(&text) {
                    debug!("Successfully parsed as generic JSON: {:?}", auth_msg);
                    if let Some(token) = auth_msg.get("token").and_then(|t| t.as_str()) {
                        debug!("Extracted token from generic auth message: {}", token);
                        jwt_validator.validate_token(token)
                    } else {
                        debug!("No 'token' field found in JSON");
                        Err("No 'token' field found in authentication message".to_string())
                    }
                } else {
                    debug!("Failed to parse as generic JSON");
                    Err("Invalid JSON format in authentication message".to_string())
                }
            }
            Ok(Message::Close(_)) => Err("Connection closed during authentication".to_string()),
            Ok(_) => Err("Invalid authentication message format".to_string()),
            Err(e) => Err(format!("WebSocket error during authentication: {}", e)),
        }
    } else {
        Err("No authentication message received".to_string())
    }
}

async fn handle_client_message(
    text: &str,
    user: &crate::auth::AuthenticatedUser,
    connection_id: Uuid,
    room_manager: &RoomManager,
    sfu: &Option<Arc<RheomeshSfu>>,
    tx: &mpsc::UnboundedSender<Message>,
) -> Result<(), String> {
    debug!("Received message from user {}: {}", user.user_id, text);

    let client_message: ClientMessage =
        serde_json::from_str(text).map_err(|e| format!("Invalid JSON: {}", e))?;

    match client_message {
        ClientMessage::Auth { .. } => {
            let error_msg = ServerMessage::error("Authentication already completed");
            send_message(tx, error_msg)?;
        }

        ClientMessage::JoinRoom {
            room_name,
            password: _,
        } => {
            let participant = RoomParticipant {
                user: user.clone(),
                connection_id,
                sender: tx.clone(),
                publish_transport: None,
                subscribe_transport: None,
            };

            match room_manager.join_room(room_name.clone(), participant).await {
                Ok(existing_participants) => {
                    let join_msg = ServerMessage::RoomJoined {
                        room_name,
                        user_id: user.user_id,
                        participants: existing_participants,
                    };
                    send_message(tx, join_msg)?;
                }
                Err(e) => {
                    let error_msg = ServerMessage::error(format!("Failed to join room: {}", e));
                    send_message(tx, error_msg)?;
                }
            }
        }

        ClientMessage::LeaveRoom { room_name } => {
            match room_manager.leave_room(&room_name, user.user_id).await {
                Ok(()) => {
                    let leave_msg = ServerMessage::RoomLeft {
                        room_name,
                        user_id: user.user_id,
                    };
                    send_message(tx, leave_msg)?;
                }
                Err(e) => {
                    let error_msg = ServerMessage::error(format!("Failed to leave room: {}", e));
                    send_message(tx, error_msg)?;
                }
            }
        }

        ClientMessage::Offer {
            room_name,
            sdp,
            target_user_id,
        } => {
            if !room_manager.user_in_room(&room_name, user.user_id).await {
                let error_msg = ServerMessage::error("You are not in this room");
                send_message(tx, error_msg)?;
                return Ok(());
            }

            // SFU integration: handle offer via publish_transport
            let rooms = room_manager.get_rooms();
            let rooms_guard = rooms.read().await;
            if let Some(room) = rooms_guard.get(&room_name) {
                if let Some(participant) = room.participants.get(&user.user_id) {
                    if let Some(publish_transport) = &participant.publish_transport {
                        // Call get_answer on the publish_transport with SDP string
                        let offer_sdp = RTCSessionDescription::offer(sdp)
                            .map_err(|e| format!("Failed to parse offer SDP: {}", e))?;
                        let answer = publish_transport
                            .get_answer(offer_sdp)
                            .await
                            .map_err(|e| format!("SFU get_answer error: {}", e))?;
                        let answer_msg = ServerMessage::Answer {
                            room_name: room_name.clone(),
                            from_user_id: user.user_id,
                            sdp: answer.sdp,
                        };
                        send_message(tx, answer_msg)?;
                    } else {
                        let error_msg =
                            ServerMessage::error("No publish transport for participant");
                        send_message(tx, error_msg)?;
                    }
                } else {
                    let error_msg = ServerMessage::error("Participant not found in room");
                    send_message(tx, error_msg)?;
                }
            } else {
                let error_msg = ServerMessage::error("Room not found");
                send_message(tx, error_msg)?;
            }
        }

        ClientMessage::Answer {
            room_name,
            sdp,
            target_user_id,
        } => {
            if !room_manager.user_in_room(&room_name, user.user_id).await {
                let error_msg = ServerMessage::error("You are not in this room");
                send_message(tx, error_msg)?;
                return Ok(());
            }
            // SFU integration: set_answer on subscribe_transport
            let rooms = room_manager.get_rooms();
            let rooms_guard = rooms.read().await;
            if let Some(room) = rooms_guard.get(&room_name) {
                if let Some(participant) = room.participants.get(&user.user_id) {
                    if let Some(subscribe_transport) = &participant.subscribe_transport {
                        let answer_sdp = RTCSessionDescription::answer(sdp)
                            .map_err(|e| format!("Failed to parse answer SDP: {}", e))?;
                        subscribe_transport
                            .set_answer(answer_sdp)
                            .await
                            .map_err(|e| format!("SFU set_answer error: {}", e))?;
                        // Optionally send a confirmation
                    } else {
                        let error_msg =
                            ServerMessage::error("No subscribe transport for participant");
                        send_message(tx, error_msg)?;
                    }
                } else {
                    let error_msg = ServerMessage::error("Participant not found in room");
                    send_message(tx, error_msg)?;
                }
            } else {
                let error_msg = ServerMessage::error("Room not found");
                send_message(tx, error_msg)?;
            }
        }

        ClientMessage::IceCandidate {
            room_name,
            candidate,
            sdp_mid: _,
            sdp_mline_index: _,
            target_user_id: _,
        } => {
            if !room_manager.user_in_room(&room_name, user.user_id).await {
                let error_msg = ServerMessage::error("You are not in this room");
                send_message(tx, error_msg)?;
                return Ok(());
            }
            // SFU integration: add_ice_candidate to both transports
            let rooms = room_manager.get_rooms();
            let rooms_guard = rooms.read().await;
            if let Some(room) = rooms_guard.get(&room_name) {
                if let Some(participant) = room.participants.get(&user.user_id) {
                    let mut ok = false;
                    if let Some(publish_transport) = &participant.publish_transport {
                        if publish_transport
                            .add_ice_candidate(&candidate)
                            .await
                            .is_ok()
                        {
                            ok = true;
                        }
                    }
                    if let Some(subscribe_transport) = &participant.subscribe_transport {
                        if subscribe_transport
                            .add_ice_candidate(&candidate)
                            .await
                            .is_ok()
                        {
                            ok = true;
                        }
                    }
                    if !ok {
                        let error_msg =
                            ServerMessage::error("Failed to add ICE candidate to any transport");
                        send_message(tx, error_msg)?;
                    }
                } else {
                    let error_msg = ServerMessage::error("Participant not found in room");
                    send_message(tx, error_msg)?;
                }
            } else {
                let error_msg = ServerMessage::error("Room not found");
                send_message(tx, error_msg)?;
            }
        }

        // SFU-specific message handlers
        ClientMessage::PublishOffer { room_name, offer } => {
            if let Some(sfu) = sfu {
                match sfu
                    .add_participant(&room_name, user.user_id as i32, user.username.clone())
                    .await
                {
                    Ok(_) => {
                        let offer_sdp = RTCSessionDescription::offer(offer)
                            .map_err(|e| format!("Failed to parse offer SDP: {}", e))?;

                        match sfu
                            .handle_publish_offer(&room_name, user.user_id as i32, offer_sdp)
                            .await
                        {
                            Ok(answer) => {
                                let answer_msg = ServerMessage::PublishAnswer {
                                    room_name,
                                    answer: answer.sdp,
                                };
                                send_message(tx, answer_msg)?;

                                // Notify other participants about new publisher
                                if let Ok(participants) =
                                    sfu.get_room_participants(&room_name).await
                                {
                                    let new_publisher_msg = ServerMessage::NewPublisher {
                                        room_name: room_name.clone(),
                                        publisher_id: user.user_id,
                                        username: user.username.clone(),
                                    };
                                    // Broadcast to room participants (implementation needed)
                                    // For now, just send to self
                                    send_message(tx, new_publisher_msg)?;
                                }
                            }
                            Err(e) => {
                                let error_msg =
                                    ServerMessage::error(format!("SFU publish offer error: {}", e));
                                send_message(tx, error_msg)?;
                            }
                        }
                    }
                    Err(e) => {
                        let error_msg =
                            ServerMessage::error(format!("Failed to add participant: {}", e));
                        send_message(tx, error_msg)?;
                    }
                }
            } else {
                let error_msg = ServerMessage::error("SFU not available");
                send_message(tx, error_msg)?;
            }
        }

        ClientMessage::PublishIceCandidate {
            room_name,
            candidate,
        } => {
            if let Some(sfu) = sfu {
                match sfu
                    .handle_publish_ice_candidate(&room_name, user.user_id as i32, &candidate)
                    .await
                {
                    Ok(_) => {
                        let ice_msg = ServerMessage::PublishIceCandidate {
                            room_name,
                            candidate,
                        };
                        send_message(tx, ice_msg)?;
                    }
                    Err(e) => {
                        let error_msg =
                            ServerMessage::error(format!("SFU publish ICE candidate error: {}", e));
                        send_message(tx, error_msg)?;
                    }
                }
            } else {
                let error_msg = ServerMessage::error("SFU not available");
                send_message(tx, error_msg)?;
            }
        }

        ClientMessage::SubscribeRequest {
            room_name,
            publisher_id,
        } => {
            if let Some(sfu) = sfu {
                match sfu
                    .handle_subscribe_request(&room_name, user.user_id as i32, publisher_id as i32)
                    .await
                {
                    Ok(offer) => {
                        let offer_msg = ServerMessage::SubscribeOffer {
                            room_name,
                            publisher_id,
                            offer: offer.sdp,
                        };
                        send_message(tx, offer_msg)?;
                    }
                    Err(e) => {
                        let error_msg =
                            ServerMessage::error(format!("SFU subscribe request error: {}", e));
                        send_message(tx, error_msg)?;
                    }
                }
            } else {
                let error_msg = ServerMessage::error("SFU not available");
                send_message(tx, error_msg)?;
            }
        }

        ClientMessage::SubscribeAnswer {
            room_name,
            publisher_id,
            answer,
        } => {
            if let Some(sfu) = sfu {
                let answer_sdp = RTCSessionDescription::answer(answer)
                    .map_err(|e| format!("Failed to parse answer SDP: {}", e))?;

                match sfu
                    .handle_subscribe_answer(
                        &room_name,
                        user.user_id as i32,
                        publisher_id as i32,
                        answer_sdp,
                    )
                    .await
                {
                    Ok(_) => {
                        // Subscription established successfully
                        debug!(
                            "Subscription established for user {} to publisher {}",
                            user.user_id, publisher_id
                        );
                    }
                    Err(e) => {
                        let error_msg =
                            ServerMessage::error(format!("SFU subscribe answer error: {}", e));
                        send_message(tx, error_msg)?;
                    }
                }
            } else {
                let error_msg = ServerMessage::error("SFU not available");
                send_message(tx, error_msg)?;
            }
        }

        ClientMessage::SubscribeIceCandidate {
            room_name,
            publisher_id,
            candidate,
        } => {
            if let Some(sfu) = sfu {
                match sfu
                    .handle_subscribe_ice_candidate(
                        &room_name,
                        user.user_id as i32,
                        publisher_id as i32,
                        &candidate,
                    )
                    .await
                {
                    Ok(_) => {
                        let ice_msg = ServerMessage::SubscribeIceCandidate {
                            room_name,
                            publisher_id,
                            candidate,
                        };
                        send_message(tx, ice_msg)?;
                    }
                    Err(e) => {
                        let error_msg = ServerMessage::error(format!(
                            "SFU subscribe ICE candidate error: {}",
                            e
                        ));
                        send_message(tx, error_msg)?;
                    }
                }
            } else {
                let error_msg = ServerMessage::error("SFU not available");
                send_message(tx, error_msg)?;
            }
        }
    }

    Ok(())
}

fn send_message(tx: &mpsc::UnboundedSender<Message>, msg: ServerMessage) -> Result<(), String> {
    let json =
        serde_json::to_string(&msg).map_err(|e| format!("Failed to serialize message: {}", e))?;

    tx.send(Message::Text(json))
        .map_err(|e| format!("Failed to send message: {}", e))?;

    Ok(())
}
