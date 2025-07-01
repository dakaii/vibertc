use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::messages::{Participant, ServerMessage};

#[derive(Debug, Clone)]
pub struct RoomParticipant {
    pub user: AuthenticatedUser,
    pub connection_id: Uuid,
    pub sender: mpsc::UnboundedSender<Message>,
}

#[derive(Debug)]
pub struct Room {
    pub name: String,
    pub participants: HashMap<u32, RoomParticipant>, // user_id -> participant
}

impl Room {
    pub fn new(name: String) -> Self {
        Self {
            name,
            participants: HashMap::new(),
        }
    }

    pub fn add_participant(&mut self, participant: RoomParticipant) -> bool {
        let user_id = participant.user.user_id;
        if self.participants.contains_key(&user_id) {
            warn!("User {} already in room {}", user_id, self.name);
            return false;
        }

        info!(
            "User {} ({}) joined room {}",
            user_id, participant.user.username, self.name
        );
        self.participants.insert(user_id, participant);
        true
    }

    pub fn remove_participant(&mut self, user_id: u32) -> Option<RoomParticipant> {
        if let Some(participant) = self.participants.remove(&user_id) {
            info!(
                "User {} ({}) left room {}",
                user_id, participant.user.username, self.name
            );
            Some(participant)
        } else {
            None
        }
    }

    pub fn get_participants_list(&self) -> Vec<Participant> {
        self.participants
            .values()
            .map(|p| Participant {
                user_id: p.user.user_id,
                username: p.user.username.clone(),
            })
            .collect()
    }

    pub fn broadcast_to_others(&self, sender_id: u32, message: ServerMessage) {
        let json_message = match serde_json::to_string(&message) {
            Ok(json) => Message::Text(json),
            Err(e) => {
                warn!("Failed to serialize message: {}", e);
                return;
            }
        };

        for (user_id, participant) in &self.participants {
            if *user_id != sender_id {
                if let Err(e) = participant.sender.send(json_message.clone()) {
                    warn!("Failed to send message to user {}: {}", user_id, e);
                }
            }
        }
    }

    pub fn broadcast_to_all(&self, message: ServerMessage) {
        let json_message = match serde_json::to_string(&message) {
            Ok(json) => Message::Text(json),
            Err(e) => {
                warn!("Failed to serialize message: {}", e);
                return;
            }
        };

        for (user_id, participant) in &self.participants {
            if let Err(e) = participant.sender.send(json_message.clone()) {
                warn!("Failed to send message to user {}: {}", user_id, e);
            }
        }
    }

    pub fn send_to_user(&self, user_id: u32, message: ServerMessage) {
        if let Some(participant) = self.participants.get(&user_id) {
            let json_message = match serde_json::to_string(&message) {
                Ok(json) => Message::Text(json),
                Err(e) => {
                    warn!("Failed to serialize message: {}", e);
                    return;
                }
            };

            if let Err(e) = participant.sender.send(json_message) {
                warn!("Failed to send message to user {}: {}", user_id, e);
            }
        }
    }

    pub fn has_participant(&self, user_id: u32) -> bool {
        self.participants.contains_key(&user_id)
    }

    pub fn is_empty(&self) -> bool {
        self.participants.is_empty()
    }
}

// Legacy type alias for backward compatibility
pub type Rooms = Arc<RwLock<HashMap<String, Room>>>;

// New trait-based system for room management
#[async_trait::async_trait]
pub trait RoomManagerTrait: Send + Sync {
    async fn join_room(
        &self,
        room_name: String,
        participant: RoomParticipant,
    ) -> Result<Vec<Participant>, String>;
    async fn leave_room(&self, room_name: &str, user_id: u32) -> Result<(), String>;
    async fn broadcast_to_room(
        &self,
        room_name: &str,
        sender_id: u32,
        message: ServerMessage,
    ) -> Result<(), String>;
    async fn send_to_user_in_room(
        &self,
        room_name: &str,
        target_user_id: u32,
        message: ServerMessage,
    ) -> Result<(), String>;
    async fn user_in_room(&self, room_name: &str, user_id: u32) -> bool;
    async fn remove_user_from_all_rooms(&self, user_id: u32, connection_id: Uuid);
    async fn get_room_participants(&self, room_name: &str) -> Vec<Participant>;
    async fn health_check(&self) -> bool;

    // For testing purposes - get access to internal room state
    fn get_rooms_for_testing(&self) -> Option<Rooms> {
        None // Default implementation returns None
    }
}

// Local implementation (existing behavior)
pub struct LocalRoomManager {
    rooms: Rooms,
}

impl Default for LocalRoomManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalRoomManager {
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn get_rooms(&self) -> Rooms {
        self.rooms.clone()
    }
}

#[async_trait::async_trait]
impl RoomManagerTrait for LocalRoomManager {
    async fn join_room(
        &self,
        room_name: String,
        participant: RoomParticipant,
    ) -> Result<Vec<Participant>, String> {
        let mut rooms = self.rooms.write().await;
        let room = rooms
            .entry(room_name.clone())
            .or_insert_with(|| Room::new(room_name.clone()));

        let existing_participants = room.get_participants_list();

        if room.add_participant(participant.clone()) {
            // Notify other participants about the new user
            let user_joined_msg = ServerMessage::UserJoined {
                room_name: room_name.clone(),
                user: Participant {
                    user_id: participant.user.user_id,
                    username: participant.user.username.clone(),
                },
            };
            room.broadcast_to_others(participant.user.user_id, user_joined_msg);

            Ok(existing_participants)
        } else {
            Err("User already in room".to_string())
        }
    }

    async fn leave_room(&self, room_name: &str, user_id: u32) -> Result<(), String> {
        let mut rooms = self.rooms.write().await;

        if let Some(room) = rooms.get_mut(room_name) {
            if let Some(_participant) = room.remove_participant(user_id) {
                // Notify other participants about the user leaving
                let user_left_msg = ServerMessage::UserLeft {
                    room_name: room_name.to_string(),
                    user_id,
                };
                room.broadcast_to_all(user_left_msg);

                // Remove empty rooms
                if room.is_empty() {
                    rooms.remove(room_name);
                    debug!("Removed empty room: {}", room_name);
                }

                Ok(())
            } else {
                Err("User not in room".to_string())
            }
        } else {
            Err("Room not found".to_string())
        }
    }

    async fn broadcast_to_room(
        &self,
        room_name: &str,
        sender_id: u32,
        message: ServerMessage,
    ) -> Result<(), String> {
        let rooms = self.rooms.read().await;

        if let Some(room) = rooms.get(room_name) {
            room.broadcast_to_others(sender_id, message);
            Ok(())
        } else {
            Err("Room not found".to_string())
        }
    }

    async fn send_to_user_in_room(
        &self,
        room_name: &str,
        target_user_id: u32,
        message: ServerMessage,
    ) -> Result<(), String> {
        let rooms = self.rooms.read().await;

        if let Some(room) = rooms.get(room_name) {
            room.send_to_user(target_user_id, message);
            Ok(())
        } else {
            Err("Room not found".to_string())
        }
    }

    async fn user_in_room(&self, room_name: &str, user_id: u32) -> bool {
        let rooms = self.rooms.read().await;
        rooms
            .get(room_name)
            .map(|room| room.has_participant(user_id))
            .unwrap_or(false)
    }

    async fn remove_user_from_all_rooms(&self, user_id: u32, connection_id: Uuid) {
        let mut rooms = self.rooms.write().await;
        let mut rooms_to_remove = Vec::new();

        for (room_name, room) in rooms.iter_mut() {
            if let Some(participant) = room.participants.get(&user_id) {
                if participant.connection_id == connection_id {
                    room.remove_participant(user_id);

                    // Notify other participants
                    let user_left_msg = ServerMessage::UserLeft {
                        room_name: room_name.clone(),
                        user_id,
                    };
                    room.broadcast_to_all(user_left_msg);

                    if room.is_empty() {
                        rooms_to_remove.push(room_name.clone());
                    }
                }
            }
        }

        // Remove empty rooms
        for room_name in rooms_to_remove {
            rooms.remove(&room_name);
            debug!("Removed empty room: {}", room_name);
        }
    }

    async fn get_room_participants(&self, room_name: &str) -> Vec<Participant> {
        let rooms = self.rooms.read().await;
        rooms
            .get(room_name)
            .map(|room| room.get_participants_list())
            .unwrap_or_default()
    }

    async fn health_check(&self) -> bool {
        true // Local implementation is always healthy
    }

    fn get_rooms_for_testing(&self) -> Option<Rooms> {
        Some(self.rooms.clone())
    }
}

// Legacy RoomManager for backward compatibility
pub struct RoomManager {
    pub inner: Box<dyn RoomManagerTrait>,
}

impl Default for RoomManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RoomManager {
    pub fn new() -> Self {
        Self {
            inner: Box::new(LocalRoomManager::new()),
        }
    }

    pub fn with_implementation(implementation: Box<dyn RoomManagerTrait>) -> Self {
        Self {
            inner: implementation,
        }
    }

    // For test compatibility - expose internal rooms when using LocalRoomManager
    pub fn get_rooms(&self) -> Rooms {
        // Try to get rooms from the underlying implementation
        if let Some(rooms) = self.inner.get_rooms_for_testing() {
            rooms
        } else {
            // Return empty rooms if implementation doesn't support testing
            Arc::new(RwLock::new(HashMap::new()))
        }
    }

    // Delegate methods to the trait implementation
    pub async fn join_room(
        &self,
        room_name: String,
        participant: RoomParticipant,
    ) -> Result<Vec<Participant>, String> {
        self.inner.join_room(room_name, participant).await
    }

    pub async fn leave_room(&self, room_name: &str, user_id: u32) -> Result<(), String> {
        self.inner.leave_room(room_name, user_id).await
    }

    pub async fn broadcast_to_room(
        &self,
        room_name: &str,
        sender_id: u32,
        message: ServerMessage,
    ) -> Result<(), String> {
        self.inner
            .broadcast_to_room(room_name, sender_id, message)
            .await
    }

    pub async fn send_to_user_in_room(
        &self,
        room_name: &str,
        target_user_id: u32,
        message: ServerMessage,
    ) -> Result<(), String> {
        self.inner
            .send_to_user_in_room(room_name, target_user_id, message)
            .await
    }

    pub async fn user_in_room(&self, room_name: &str, user_id: u32) -> bool {
        self.inner.user_in_room(room_name, user_id).await
    }

    pub async fn remove_user_from_all_rooms(&self, user_id: u32, connection_id: Uuid) {
        self.inner
            .remove_user_from_all_rooms(user_id, connection_id)
            .await
    }

    pub async fn get_room_participants(&self, room_name: &str) -> Vec<Participant> {
        self.inner.get_room_participants(room_name).await
    }

    pub async fn health_check(&self) -> bool {
        self.inner.health_check().await
    }
}
