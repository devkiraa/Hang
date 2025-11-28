use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Messages sent between client and server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum Message {
    // Client -> Server
    CreateRoom {
        file_hash: String,
    },
    JoinRoom {
        room_id: Uuid,
        file_hash: String,
    },
    LeaveRoom,
    SyncCommand(SyncCommand),

    // Server -> Client
    RoomCreated {
        room_id: Uuid,
        client_id: Uuid,
    },
    RoomJoined {
        room_id: Uuid,
        client_id: Uuid,
        is_host: bool,
    },
    RoomLeft,
    RoomNotFound,
    FileHashMismatch {
        expected: String,
    },
    SyncBroadcast {
        from_client: Uuid,
        command: SyncCommand,
    },
    Error {
        message: String,
    },
}

/// Synchronization commands for video playback
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum SyncCommand {
    Play { timestamp: f64 },
    Pause { timestamp: f64 },
    Seek { timestamp: f64 },
    Speed { rate: f64 },
    Stop,
}

/// Room state tracked by server
#[derive(Debug, Clone)]
pub struct Room {
    pub host_id: Uuid,
    pub file_hash: String,
}

/// Client connection metadata
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub room_id: Option<Uuid>,
}
