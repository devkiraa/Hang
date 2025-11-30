use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Messages sent between client and server (must match server protocol)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum Message {
    // Client -> Server
    CreateRoom {
        file_hash: String,
        passcode: Option<String>,
        display_name: Option<String>,
        capacity: Option<usize>,
    },
    JoinRoom {
        room_id: String,
        file_hash: String,
        passcode: Option<String>,
        display_name: Option<String>,
    },
    ResumeSession {
        token: String,
        display_name: Option<String>,
    },
    LeaveRoom,
    SyncCommand(SyncCommand),

    // Server -> Client
    RoomCreated {
        room_id: String,
        client_id: Uuid,
        passcode_enabled: bool,
        file_hash: String,
        resume_token: String,
        capacity: usize,
        display_name: String,
    },
    RoomJoined {
        room_id: String,
        client_id: Uuid,
        is_host: bool,
        passcode_enabled: bool,
        file_hash: String,
        resume_token: String,
        capacity: usize,
        display_name: String,
    },
    RoomLeft,
    RoomNotFound,
    RoomFull {
        capacity: usize,
    },
    FileHashMismatch {
        expected: String,
    },
    SyncBroadcast {
        from_client: Uuid,
        command: SyncCommand,
    },
    RoomMemberUpdate {
        room_id: String,
        members: Vec<MemberSummary>,
        capacity: usize,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberSummary {
    pub client_id: Uuid,
    pub display_name: String,
    pub is_host: bool,
}
