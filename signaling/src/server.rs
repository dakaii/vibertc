use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use uuid::Uuid;
// Removed url dependency
use anyhow::Result;
use tracing::{debug, error, info};

use crate::auth::JwtValidator;
use crate::messages::{ClientMessage, ServerMessage};
use crate::rheomesh_sfu::RheomeshSfu;
use crate::room::{RoomManager, RoomParticipant};

pub async fn start_server(host: String, port: u16, jwt_secret: String) -> Result<()> {
    let room_manager = RoomManager::new();
    start_server_with_room_manager(host, port, jwt_secret, room_manager).await
}

pub async fn start_server_with_room_manager(
    host: String,
    port: u16,
    jwt_secret: String,
    room_manager: RoomManager,
) -> Result<()> {
    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;

    info!("WebSocket server listening on: {}", addr);

    let jwt_validator = Arc::new(JwtValidator::new(&jwt_secret));
    let room_manager = Arc::new(room_manager);

    while let Ok((stream, peer_addr)) = listener.accept().await {
        info!("New connection from: {}", peer_addr);

        let jwt_validator = jwt_validator.clone();
        let room_manager = room_manager.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, jwt_validator, room_manager).await {
                error!("Connection error: {}", e);
            }
        });
    }

    Ok(())
}

pub async fn start_server_with_room_manager_and_sfu(
    host: String,
    port: u16,
    jwt_secret: String,
    room_manager: RoomManager,
    sfu: Option<Arc<RheomeshSfu>>,
) -> Result<()> {
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
            if let Err(e) =
                handle_connection_with_sfu(stream, jwt_validator, room_manager, sfu).await
            {
                error!("Connection error: {}", e);
            }
        });
    }

    Ok(())
}

async fn handle_connection_with_sfu(
    stream: TcpStream,
    jwt_validator: Arc<JwtValidator>,
    room_manager: Arc<RoomManager>,
    sfu: Option<Arc<RheomeshSfu>>,
) -> Result<()> {
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
                    if let Err(e) = handle_client_message_with_sfu(
                        &text,
                        &user,
                        connection_id,
                        &room_manager,
                        &sfu,
                        &tx,
                    )
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

        // Clean up from SFU if available
        if let Some(ref _sfu) = sfu {
            // TODO: Remove user from SFU rooms
            info!("TODO: Clean up user {} from SFU rooms", user.user_id);
        }

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

async fn handle_connection(
    stream: TcpStream,
    jwt_validator: Arc<JwtValidator>,
    room_manager: Arc<RoomManager>,
) -> Result<()> {
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
                        handle_client_message(&text, &user, connection_id, &room_manager, &tx).await
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

            let offer_msg = ServerMessage::Offer {
                room_name: room_name.clone(),
                from_user_id: user.user_id,
                sdp,
            };

            if let Some(target_id) = target_user_id {
                room_manager
                    .send_to_user_in_room(&room_name, target_id, offer_msg)
                    .await
                    .map_err(|e| format!("Failed to send offer: {}", e))?;
            } else {
                room_manager
                    .broadcast_to_room(&room_name, user.user_id, offer_msg)
                    .await
                    .map_err(|e| format!("Failed to broadcast offer: {}", e))?;
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

            let answer_msg = ServerMessage::Answer {
                room_name: room_name.clone(),
                from_user_id: user.user_id,
                sdp,
            };

            room_manager
                .send_to_user_in_room(&room_name, target_user_id, answer_msg)
                .await
                .map_err(|e| format!("Failed to send answer: {}", e))?;
        }

        ClientMessage::IceCandidate {
            room_name,
            candidate,
            sdp_mid,
            sdp_mline_index,
            target_user_id,
        } => {
            if !room_manager.user_in_room(&room_name, user.user_id).await {
                let error_msg = ServerMessage::error("You are not in this room");
                send_message(tx, error_msg)?;
                return Ok(());
            }

            let ice_msg = ServerMessage::IceCandidate {
                room_name: room_name.clone(),
                from_user_id: user.user_id,
                candidate,
                sdp_mid,
                sdp_mline_index,
            };

            if let Some(target_id) = target_user_id {
                room_manager
                    .send_to_user_in_room(&room_name, target_id, ice_msg)
                    .await
                    .map_err(|e| format!("Failed to send ICE candidate: {}", e))?;
            } else {
                room_manager
                    .broadcast_to_room(&room_name, user.user_id, ice_msg)
                    .await
                    .map_err(|e| format!("Failed to broadcast ICE candidate: {}", e))?;
            }
        }

        // SFU message handlers - placeholder for now
        ClientMessage::PublishOffer { .. } => {
            // TODO: Handle SFU publish offer
        }
        ClientMessage::PublishIceCandidate { .. } => {
            // TODO: Handle SFU publish ICE candidate
        }
        ClientMessage::SubscribeRequest { .. } => {
            // TODO: Handle SFU subscribe request
        }
        ClientMessage::SubscribeAnswer { .. } => {
            // TODO: Handle SFU subscribe answer
        }
        ClientMessage::SubscribeIceCandidate { .. } => {
            // TODO: Handle SFU subscribe ICE candidate
        }
    }

    Ok(())
}

async fn handle_client_message_with_sfu(
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
        // Handle regular signaling messages the same way
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
            };

            match room_manager.join_room(room_name.clone(), participant).await {
                Ok(existing_participants) => {
                    let join_msg = ServerMessage::RoomJoined {
                        room_name: room_name.clone(),
                        user_id: user.user_id,
                        participants: existing_participants,
                    };
                    send_message(tx, join_msg)?;

                    // Also add to SFU if available
                    if let Some(ref sfu) = sfu {
                        if let Err(e) = sfu
                            .add_participant(&room_name, user.user_id, user.username.clone())
                            .await
                        {
                            debug!("Failed to add participant to SFU: {}", e);
                        }
                    }
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
                        room_name: room_name.clone(),
                        user_id: user.user_id,
                    };
                    send_message(tx, leave_msg)?;

                    // Also remove from SFU if available
                    if let Some(ref sfu) = sfu {
                        if let Err(e) = sfu.remove_participant(&room_name, user.user_id).await {
                            debug!("Failed to remove participant from SFU: {}", e);
                        }
                    }
                }
                Err(e) => {
                    let error_msg = ServerMessage::error(format!("Failed to leave room: {}", e));
                    send_message(tx, error_msg)?;
                }
            }
        }

        // Regular WebRTC signaling (peer-to-peer)
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

            let offer_msg = ServerMessage::Offer {
                room_name: room_name.clone(),
                from_user_id: user.user_id,
                sdp,
            };

            if let Some(target_id) = target_user_id {
                room_manager
                    .send_to_user_in_room(&room_name, target_id, offer_msg)
                    .await
                    .map_err(|e| format!("Failed to send offer: {}", e))?;
            } else {
                room_manager
                    .broadcast_to_room(&room_name, user.user_id, offer_msg)
                    .await
                    .map_err(|e| format!("Failed to broadcast offer: {}", e))?;
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

            let answer_msg = ServerMessage::Answer {
                room_name: room_name.clone(),
                from_user_id: user.user_id,
                sdp,
            };

            room_manager
                .send_to_user_in_room(&room_name, target_user_id, answer_msg)
                .await
                .map_err(|e| format!("Failed to send answer: {}", e))?;
        }

        ClientMessage::IceCandidate {
            room_name,
            candidate,
            sdp_mid,
            sdp_mline_index,
            target_user_id,
        } => {
            if !room_manager.user_in_room(&room_name, user.user_id).await {
                let error_msg = ServerMessage::error("You are not in this room");
                send_message(tx, error_msg)?;
                return Ok(());
            }

            let ice_msg = ServerMessage::IceCandidate {
                room_name: room_name.clone(),
                from_user_id: user.user_id,
                candidate,
                sdp_mid,
                sdp_mline_index,
            };

            if let Some(target_id) = target_user_id {
                room_manager
                    .send_to_user_in_room(&room_name, target_id, ice_msg)
                    .await
                    .map_err(|e| format!("Failed to send ICE candidate: {}", e))?;
            } else {
                room_manager
                    .broadcast_to_room(&room_name, user.user_id, ice_msg)
                    .await
                    .map_err(|e| format!("Failed to broadcast ICE candidate: {}", e))?;
            }
        }

        // SFU message handlers - simplified placeholders for now
        ClientMessage::PublishOffer { .. } => {
            info!("TODO: Handle SFU publish offer from user {}", user.user_id);
            let error_msg = ServerMessage::error("SFU publish offer not yet implemented");
            send_message(tx, error_msg)?;
        }
        ClientMessage::PublishIceCandidate { .. } => {
            info!(
                "TODO: Handle SFU publish ICE candidate from user {}",
                user.user_id
            );
        }
        ClientMessage::SubscribeRequest { .. } => {
            info!(
                "TODO: Handle SFU subscribe request from user {}",
                user.user_id
            );
            let error_msg = ServerMessage::error("SFU subscribe request not yet implemented");
            send_message(tx, error_msg)?;
        }
        ClientMessage::SubscribeAnswer { .. } => {
            info!(
                "TODO: Handle SFU subscribe answer from user {}",
                user.user_id
            );
        }
        ClientMessage::SubscribeIceCandidate { .. } => {
            info!(
                "TODO: Handle SFU subscribe ICE candidate from user {}",
                user.user_id
            );
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
