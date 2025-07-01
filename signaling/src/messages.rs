use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMessage {
    #[serde(rename = "auth")]
    Auth { token: String },

    #[serde(rename = "join-room")]
    JoinRoom {
        #[serde(rename = "roomName")]
        room_name: String,
        password: Option<String>,
    },

    #[serde(rename = "leave-room")]
    LeaveRoom {
        #[serde(rename = "roomName")]
        room_name: String,
    },

    #[serde(rename = "offer")]
    Offer {
        #[serde(rename = "roomName")]
        room_name: String,
        sdp: String,
        #[serde(rename = "targetUserId")]
        target_user_id: Option<u32>,
    },

    #[serde(rename = "answer")]
    Answer {
        #[serde(rename = "roomName")]
        room_name: String,
        sdp: String,
        #[serde(rename = "targetUserId")]
        target_user_id: u32,
    },

    #[serde(rename = "ice-candidate")]
    IceCandidate {
        #[serde(rename = "roomName")]
        room_name: String,
        candidate: String,
        #[serde(rename = "sdpMid")]
        sdp_mid: Option<String>,
        #[serde(rename = "sdpMLineIndex")]
        sdp_mline_index: Option<u32>,
        #[serde(rename = "targetUserId")]
        target_user_id: Option<u32>,
    },

    // SFU specific messages
    #[serde(rename = "publish-offer")]
    PublishOffer {
        #[serde(rename = "roomName")]
        room_name: String,
        offer: String, // SDP offer
    },

    #[serde(rename = "publish-ice-candidate")]
    PublishIceCandidate {
        #[serde(rename = "roomName")]
        room_name: String,
        candidate: String,
    },

    #[serde(rename = "subscribe-request")]
    SubscribeRequest {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "publisherId")]
        publisher_id: u32,
    },

    #[serde(rename = "subscribe-answer")]
    SubscribeAnswer {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "publisherId")]
        publisher_id: u32,
        answer: String, // SDP answer
    },

    #[serde(rename = "subscribe-ice-candidate")]
    SubscribeIceCandidate {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "publisherId")]
        publisher_id: u32,
        candidate: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMessage {
    #[serde(rename = "room-joined")]
    RoomJoined {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "userId")]
        user_id: u32,
        participants: Vec<Participant>,
    },

    #[serde(rename = "room-left")]
    RoomLeft {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "userId")]
        user_id: u32,
    },

    #[serde(rename = "user-joined")]
    UserJoined {
        #[serde(rename = "roomName")]
        room_name: String,
        user: Participant,
    },

    #[serde(rename = "user-left")]
    UserLeft {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "userId")]
        user_id: u32,
    },

    #[serde(rename = "offer")]
    Offer {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "fromUserId")]
        from_user_id: u32,
        sdp: String,
    },

    #[serde(rename = "answer")]
    Answer {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "fromUserId")]
        from_user_id: u32,
        sdp: String,
    },

    #[serde(rename = "ice-candidate")]
    IceCandidate {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "fromUserId")]
        from_user_id: u32,
        candidate: String,
        #[serde(rename = "sdpMid")]
        sdp_mid: Option<String>,
        #[serde(rename = "sdpMLineIndex")]
        sdp_mline_index: Option<u32>,
    },

    #[serde(rename = "error")]
    Error { message: String, code: Option<u32> },

    #[serde(rename = "authenticated")]
    Authenticated {
        #[serde(rename = "userId")]
        user_id: u32,
        username: String,
    },

    // SFU specific messages
    #[serde(rename = "publish-answer")]
    PublishAnswer {
        #[serde(rename = "roomName")]
        room_name: String,
        answer: String, // SDP answer
    },

    #[serde(rename = "publish-ice-candidate")]
    PublishIceCandidate {
        #[serde(rename = "roomName")]
        room_name: String,
        candidate: String,
    },

    #[serde(rename = "subscribe-offer")]
    SubscribeOffer {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "publisherId")]
        publisher_id: u32,
        offer: String, // SDP offer
    },

    #[serde(rename = "subscribe-ice-candidate")]
    SubscribeIceCandidate {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "publisherId")]
        publisher_id: u32,
        candidate: String,
    },

    #[serde(rename = "new-publisher")]
    NewPublisher {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "publisherId")]
        publisher_id: u32,
        username: String,
    },

    #[serde(rename = "publisher-left")]
    PublisherLeft {
        #[serde(rename = "roomName")]
        room_name: String,
        #[serde(rename = "publisherId")]
        publisher_id: u32,
    },

    #[serde(rename = "publishers-list")]
    PublishersList {
        #[serde(rename = "roomName")]
        room_name: String,
        publishers: Vec<Publisher>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    #[serde(rename = "userId")]
    pub user_id: u32,
    pub username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Publisher {
    #[serde(rename = "publisherId")]
    pub publisher_id: u32,
    pub username: String,
}

impl ServerMessage {
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            code: None,
        }
    }

    pub fn error_with_code(message: impl Into<String>, code: u32) -> Self {
        Self::Error {
            message: message.into(),
            code: Some(code),
        }
    }
}
