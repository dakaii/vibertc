use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Simplified SFU room for now - will be enhanced with Rheomesh later
pub struct SfuRoom {
    pub room_id: String,
    pub participants: HashMap<u32, String>, // user_id -> username
}

impl SfuRoom {
    pub fn new(room_id: String) -> Self {
        Self {
            room_id,
            participants: HashMap::new(),
        }
    }

    pub fn add_participant(&mut self, user_id: u32, username: String) {
        self.participants.insert(user_id, username);
        info!("Added participant {} to SFU room {}", user_id, self.room_id);
    }

    pub fn remove_participant(&mut self, user_id: u32) -> Option<String> {
        let username = self.participants.remove(&user_id);
        if let Some(ref name) = username {
            info!("Removed participant {} ({}) from SFU room {}", user_id, name, self.room_id);
        }
        username
    }
}

/// Main SFU manager - simplified for now
pub struct RheomeshSfu {
    rooms: Arc<RwLock<HashMap<String, SfuRoom>>>,
}

impl RheomeshSfu {
    pub async fn new() -> Result<Self> {
        let rooms = Arc::new(RwLock::new(HashMap::new()));
        info!("🚀 Rheomesh SFU initialized successfully (simplified mode)");
        Ok(Self { rooms })
    }

    pub async fn add_participant(&self, room_id: &str, user_id: u32, username: String) -> Result<()> {
        let mut rooms = self.rooms.write().await;
        let room = rooms.entry(room_id.to_string()).or_insert_with(|| SfuRoom::new(room_id.to_string()));
        room.add_participant(user_id, username);
        Ok(())
    }

    pub async fn remove_participant(&self, room_id: &str, user_id: u32) -> Result<Option<String>> {
        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get_mut(room_id) {
            let username = room.remove_participant(user_id);
            if room.participants.is_empty() {
                rooms.remove(room_id);
            }
            return Ok(username);
        }
        Ok(None)
    }
}
