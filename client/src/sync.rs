use anyhow::{Context, Result};
use directories::ProjectDirs;
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use uuid::Uuid;

use crate::protocol::{Message, SyncCommand};

pub struct SyncClient {
    tx: Mutex<Option<mpsc::UnboundedSender<Message>>>,
    room_id: Mutex<Option<String>>,
    client_id: Mutex<Option<Uuid>>,
    is_host: Mutex<bool>,
    session_store: SessionStore,
}

impl SyncClient {
    pub fn new() -> Self {
        Self {
            tx: Mutex::new(None),
            room_id: Mutex::new(None),
            client_id: Mutex::new(None),
            is_host: Mutex::new(false),
            session_store: SessionStore::new(),
        }
    }

    /// Connect to the sync server
    pub async fn connect<F>(&self, server_url: &str, on_message: F) -> Result<()>
    where
        F: Fn(Message) + Send + 'static,
    {
        let (ws_stream, _) = connect_async(server_url)
            .await
            .context("Failed to connect to server")?;

        let (mut ws_sender, mut ws_receiver) = ws_stream.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

        *self.tx.lock() = Some(tx);

        // Spawn send task
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Ok(json) = serde_json::to_string(&msg) {
                    if ws_sender.send(WsMessage::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
        });

        // Spawn receive task
        tokio::spawn(async move {
            while let Some(msg) = ws_receiver.next().await {
                if let Ok(WsMessage::Text(text)) = msg {
                    if let Ok(parsed) = serde_json::from_str::<Message>(&text) {
                        on_message(parsed);
                    }
                }
            }
        });

        Ok(())
    }

    /// Create a new room
    pub fn create_room(&self, file_hash: String, passcode: Option<String>) -> Result<()> {
        self.send_message(Message::CreateRoom {
            file_hash,
            passcode,
        })
    }

    /// Join an existing room
    pub fn join_room(
        &self,
        room_id: String,
        file_hash: String,
        passcode: Option<String>,
    ) -> Result<()> {
        self.send_message(Message::JoinRoom {
            room_id,
            file_hash,
            passcode,
        })
    }

    /// Leave current room
    pub fn leave_room(&self) -> Result<()> {
        self.send_message(Message::LeaveRoom)
    }

    /// Attempt to resume a room using a server-issued token
    pub fn resume_session(&self, token: String) -> Result<()> {
        self.send_message(Message::ResumeSession { token })
    }

    /// Send a sync command
    pub fn send_sync_command(&self, command: SyncCommand) -> Result<()> {
        self.send_message(Message::SyncCommand(command))
    }

    /// Update room state after receiving server response
    pub fn set_room_joined(&self, room_id: String, client_id: Uuid, is_host: bool) {
        *self.room_id.lock() = Some(room_id);
        *self.client_id.lock() = Some(client_id);
        *self.is_host.lock() = is_host;
    }

    /// Clear room state
    pub fn clear_room(&self) {
        *self.room_id.lock() = None;
        *self.client_id.lock() = None;
        *self.is_host.lock() = false;
    }

    /// Persist the latest successful session locally
    pub fn persist_session(&self, session: &PersistedSession) -> Result<()> {
        self.session_store.save(session)
    }

    /// Remove any locally cached session token
    pub fn clear_persisted_session(&self) -> Result<()> {
        self.session_store.clear();
        Ok(())
    }

    /// Fetch the most recently cached session (if any)
    pub fn saved_session(&self) -> Option<PersistedSession> {
        self.session_store.load()
    }

    fn send_message(&self, msg: Message) -> Result<()> {
        if let Some(tx) = self.tx.lock().as_ref() {
            tx.send(msg).context("Failed to send message")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    pub room_id: String,
    pub resume_token: String,
    pub file_hash: String,
    pub is_host: bool,
}

struct SessionStore {
    path: PathBuf,
    cached: Mutex<Option<PersistedSession>>,
}

impl SessionStore {
    fn new() -> Self {
        let path = Self::resolve_path();
        let cached = Self::read_from_disk(&path);
        Self {
            path,
            cached: Mutex::new(cached),
        }
    }

    fn load(&self) -> Option<PersistedSession> {
        self.cached.lock().clone()
    }

    fn save(&self, session: &PersistedSession) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).context("Failed to create session cache directory")?;
        }
        let payload = serde_json::to_vec_pretty(session)?;
        fs::write(&self.path, payload).context("Failed to write session cache")?;
        *self.cached.lock() = Some(session.clone());
        Ok(())
    }

    fn clear(&self) {
        let _ = fs::remove_file(&self.path);
        *self.cached.lock() = None;
    }

    fn read_from_disk(path: &PathBuf) -> Option<PersistedSession> {
        fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
    }

    fn resolve_path() -> PathBuf {
        if let Some(dirs) = ProjectDirs::from("com", "hang", "Hang") {
            let data_dir = dirs.data_dir();
            let _ = fs::create_dir_all(data_dir);
            data_dir.join("session.json")
        } else {
            env::temp_dir().join("hang-session.json")
        }
    }
}
