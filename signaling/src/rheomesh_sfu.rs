use anyhow::Result;
use rheomesh::{
    config::{MediaConfig, WebRTCTransportConfig},
    publish_transport::PublishTransport,
    router::Router,
    subscribe_transport::SubscribeTransport,
    worker::Worker,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

/// WebRTC offer/answer/ICE candidate messages for the SFU
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SfuMessage {
    // Publisher messages
    PublishOffer {
        user_id: i32,
        room_id: String,
        offer: RTCSessionDescription,
    },
    PublishAnswer {
        user_id: i32,
        room_id: String,
        answer: RTCSessionDescription,
    },
    PublishIceCandidate {
        user_id: i32,
        room_id: String,
        candidate: String,
    },

    // Subscriber messages
    SubscribeRequest {
        user_id: i32,
        room_id: String,
        publisher_id: i32,
    },
    SubscribeOffer {
        user_id: i32,
        room_id: String,
        publisher_id: i32,
        offer: RTCSessionDescription,
    },
    SubscribeAnswer {
        user_id: i32,
        room_id: String,
        publisher_id: i32,
        answer: RTCSessionDescription,
    },
    SubscribeIceCandidate {
        user_id: i32,
        room_id: String,
        publisher_id: i32,
        candidate: String,
    },

    // SFU responses
    PublishReady {
        user_id: i32,
        room_id: String,
        publisher_id: String,
    },
    SubscribeReady {
        user_id: i32,
        room_id: String,
        publisher_id: i32,
        subscriber_id: String,
    },
    NewPublisher {
        room_id: String,
        publisher_id: i32,
        username: String,
    },
    PublisherLeft {
        room_id: String,
        publisher_id: i32,
    },
    Error {
        message: String,
    },
}

/// Represents a room with its Rheomesh router and connected users
pub struct SfuRoom {
    pub room_id: String,
    pub router: Router,
    pub publishers: HashMap<i32, PublishTransport>, // user_id -> PublishTransport
    pub subscribers: HashMap<i32, HashMap<i32, SubscribeTransport>>, // subscriber_id -> (publisher_id -> SubscribeTransport)
    pub participants: HashMap<i32, String>,                          // user_id -> username
}

impl SfuRoom {
    pub async fn new(room_id: String) -> Result<Self> {
        let media_config = MediaConfig::default();
        let router = Router::new(media_config);

        Ok(Self {
            room_id,
            router,
            publishers: HashMap::new(),
            subscribers: HashMap::new(),
            participants: HashMap::new(),
        })
    }

    pub fn get_webrtc_config() -> WebRTCTransportConfig {
        let mut config = WebRTCTransportConfig::default();
        config.configuration.ice_servers = vec![
            RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                ..Default::default()
            },
            RTCIceServer {
                urls: vec!["stun:stun1.l.google.com:19302".to_owned()],
                ..Default::default()
            },
        ];
        config
    }

    pub async fn add_participant(&mut self, user_id: i32, username: String) {
        self.participants.insert(user_id, username);
        info!("Added participant {} to room {}", user_id, self.room_id);
    }

    pub async fn remove_participant(&mut self, user_id: i32) -> Option<String> {
        // Clean up publisher
        if let Some(_) = self.publishers.remove(&user_id) {
            info!("Removed publisher {} from room {}", user_id, self.room_id);
        }

        // Clean up all subscriptions for this user
        self.subscribers.remove(&user_id);

        // Clean up subscriptions TO this user (if they were a publisher)
        for (_, publisher_subs) in self.subscribers.iter_mut() {
            publisher_subs.remove(&user_id);
        }

        // Remove from participants
        let username = self.participants.remove(&user_id);
        if let Some(ref name) = username {
            info!(
                "Removed participant {} ({}) from room {}",
                user_id, name, self.room_id
            );
        }

        username
    }

    pub async fn create_publish_transport(
        &mut self,
        user_id: i32,
    ) -> Result<&mut PublishTransport> {
        let config = Self::get_webrtc_config();
        let transport = self.router.create_publish_transport(config).await;

        self.publishers.insert(user_id, transport);
        Ok(self.publishers.get_mut(&user_id).unwrap())
    }

    pub async fn create_subscribe_transport(
        &mut self,
        subscriber_id: i32,
        publisher_id: i32,
    ) -> Result<&mut SubscribeTransport> {
        let config = Self::get_webrtc_config();
        let transport = self.router.create_subscribe_transport(config).await;

        self.subscribers
            .entry(subscriber_id)
            .or_insert_with(HashMap::new)
            .insert(publisher_id, transport);

        Ok(self
            .subscribers
            .get_mut(&subscriber_id)
            .unwrap()
            .get_mut(&publisher_id)
            .unwrap())
    }

    pub fn get_publish_transport(&mut self, user_id: i32) -> Option<&mut PublishTransport> {
        self.publishers.get_mut(&user_id)
    }

    pub fn get_subscribe_transport(
        &mut self,
        subscriber_id: i32,
        publisher_id: i32,
    ) -> Option<&mut SubscribeTransport> {
        self.subscribers
            .get_mut(&subscriber_id)?
            .get_mut(&publisher_id)
    }

    pub fn get_publisher_ids(&self) -> Vec<i32> {
        self.publishers.keys().cloned().collect()
    }

    pub fn get_participant_name(&self, user_id: i32) -> Option<&String> {
        self.participants.get(&user_id)
    }
}

/// Main SFU manager that handles multiple rooms using Rheomesh
pub struct RheomeshSfu {
    worker: Worker,
    rooms: Arc<RwLock<HashMap<String, SfuRoom>>>,
}

impl RheomeshSfu {
    pub async fn new() -> Result<Self> {
        let worker = Worker::new();
        let rooms = Arc::new(RwLock::new(HashMap::new()));

        info!("🚀 Rheomesh SFU initialized successfully");

        Ok(Self { worker, rooms })
    }

    pub async fn get_or_create_room(&self, room_id: &str) -> Result<()> {
        let mut rooms = self.rooms.write().await;

        if !rooms.contains_key(room_id) {
            let room = SfuRoom::new(room_id.to_string()).await?;
            rooms.insert(room_id.to_string(), room);
            info!("Created new SFU room: {}", room_id);
        }

        Ok(())
    }

    pub async fn add_participant(
        &self,
        room_id: &str,
        user_id: i32,
        username: String,
    ) -> Result<()> {
        self.get_or_create_room(room_id).await?;

        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get_mut(room_id) {
            room.add_participant(user_id, username).await;
        }

        Ok(())
    }

    pub async fn remove_participant(&self, room_id: &str, user_id: i32) -> Result<Option<String>> {
        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get_mut(room_id) {
            let username = room.remove_participant(user_id).await;

            // Remove room if no participants left
            if room.participants.is_empty() {
                rooms.remove(room_id);
                info!("Removed empty room: {}", room_id);
            }

            return Ok(username);
        }

        Ok(None)
    }

    pub async fn handle_publish_offer(
        &self,
        room_id: &str,
        user_id: i32,
        offer: RTCSessionDescription,
    ) -> Result<RTCSessionDescription> {
        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get_mut(room_id) {
            let transport = room.create_publish_transport(user_id).await?;
            let answer = transport.get_answer(offer).await?;

            info!(
                "Created publish transport for user {} in room {}",
                user_id, room_id
            );
            return Ok(answer);
        }

        Err(anyhow::anyhow!("Room not found: {}", room_id))
    }

    pub async fn handle_publish_ice_candidate(
        &self,
        room_id: &str,
        user_id: i32,
        candidate: &str,
    ) -> Result<()> {
        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get_mut(room_id) {
            if let Some(transport) = room.get_publish_transport(user_id) {
                // Parse the ICE candidate string and add it
                // Note: This is a simplified version - you might need to parse the candidate properly
                debug!(
                    "Adding ICE candidate for publisher {} in room {}: {}",
                    user_id, room_id, candidate
                );
                // transport.add_ice_candidate(candidate).await?;
                return Ok(());
            }
        }

        Err(anyhow::anyhow!(
            "Publisher transport not found for user {} in room {}",
            user_id,
            room_id
        ))
    }

    pub async fn handle_subscribe_request(
        &self,
        room_id: &str,
        subscriber_id: i32,
        publisher_id: i32,
    ) -> Result<RTCSessionDescription> {
        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get_mut(room_id) {
            // Check if publisher exists
            if !room.publishers.contains_key(&publisher_id) {
                return Err(anyhow::anyhow!(
                    "Publisher {} not found in room {}",
                    publisher_id,
                    room_id
                ));
            }

            let transport = room
                .create_subscribe_transport(subscriber_id, publisher_id)
                .await?;
            let (_, offer) = transport.subscribe(publisher_id.to_string()).await?;

            info!(
                "Created subscription from user {} to publisher {} in room {}",
                subscriber_id, publisher_id, room_id
            );
            return Ok(offer);
        }

        Err(anyhow::anyhow!("Room not found: {}", room_id))
    }

    pub async fn handle_subscribe_answer(
        &self,
        room_id: &str,
        subscriber_id: i32,
        publisher_id: i32,
        answer: RTCSessionDescription,
    ) -> Result<()> {
        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get_mut(room_id) {
            if let Some(transport) = room.get_subscribe_transport(subscriber_id, publisher_id) {
                transport.set_answer(answer).await?;
                info!(
                    "Set answer for subscription from user {} to publisher {} in room {}",
                    subscriber_id, publisher_id, room_id
                );
                return Ok(());
            }
        }

        Err(anyhow::anyhow!("Subscriber transport not found"))
    }

    pub async fn handle_subscribe_ice_candidate(
        &self,
        room_id: &str,
        subscriber_id: i32,
        publisher_id: i32,
        candidate: &str,
    ) -> Result<()> {
        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get_mut(room_id) {
            if let Some(transport) = room.get_subscribe_transport(subscriber_id, publisher_id) {
                debug!(
                    "Adding ICE candidate for subscriber {} -> publisher {} in room {}: {}",
                    subscriber_id, publisher_id, room_id, candidate
                );
                // transport.add_ice_candidate(candidate).await?;
                return Ok(());
            }
        }

        Err(anyhow::anyhow!("Subscriber transport not found"))
    }

    pub async fn get_room_publishers(&self, room_id: &str) -> Result<Vec<(i32, String)>> {
        let rooms = self.rooms.read().await;
        if let Some(room) = rooms.get(room_id) {
            let publishers: Vec<(i32, String)> = room
                .get_publisher_ids()
                .into_iter()
                .filter_map(|id| room.get_participant_name(id).map(|name| (id, name.clone())))
                .collect();
            return Ok(publishers);
        }

        Ok(Vec::new())
    }

    pub async fn get_room_participants(&self, room_id: &str) -> Result<Vec<(i32, String)>> {
        let rooms = self.rooms.read().await;
        if let Some(room) = rooms.get(room_id) {
            let participants: Vec<(i32, String)> = room
                .participants
                .iter()
                .map(|(id, name)| (*id, name.clone()))
                .collect();
            return Ok(participants);
        }

        Ok(Vec::new())
    }
}
