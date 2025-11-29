use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Messages sent between client and server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum Message {
    // Client -> Server
    CreateRoom {
        file_hash: String,
        passcode: Option<String>,
    },
    JoinRoom {
        room_id: String,
        file_hash: String,
        passcode: Option<String>,
    },
    LeaveRoom,
    SyncCommand(SyncCommand),

    // Server -> Client
    RoomCreated {
        room_id: String,
        client_id: Uuid,
        passcode_enabled: bool,
    },
    RoomJoined {
        room_id: String,
        client_id: Uuid,
        is_host: bool,
        passcode_enabled: bool,
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
    RoomMemberUpdate {
        room_id: String,
        members: usize,
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
    pub passcode_hash: Option<String>,
}

/// Client connection metadata
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub room_id: Option<String>,
}
