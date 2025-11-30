use anyhow::{Context, Result};
use directories::ProjectDirs;
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot},
    time::sleep,
};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use uuid::Uuid;

use crate::protocol::{Message, SyncCommand};

pub struct SyncClient {
    inner: Arc<SyncClientState>,
}

struct SyncClientState {
    tx: Mutex<Option<mpsc::UnboundedSender<WsMessage>>>,
    room_id: Mutex<Option<String>>,
    client_id: Mutex<Option<Uuid>>,
    is_host: Mutex<bool>,
    session_store: SessionStore,
    stats: Mutex<SyncStats>,
}

#[derive(Default, Clone)]
struct SyncStats {
    bytes_out: u64,
    bytes_in: u64,
    messages_out: u64,
    messages_in: u64,
    last_message_at: Option<Instant>,
    last_ping_sent: Option<Instant>,
    last_ping_nonce: Option<u64>,
    last_rtt_ms: Option<f32>,
    last_disconnect_at: Option<Instant>,
    reconnect_attempts: u32,
    connected_since: Option<Instant>,
    endpoint_label: Option<String>,
}

pub struct SyncStatsSnapshot {
    pub bytes_out: u64,
    pub bytes_in: u64,
    pub messages_out: u64,
    pub messages_in: u64,
    pub last_rtt_ms: Option<f32>,
    pub last_message_age: Option<f32>,
    pub connected_duration: Option<f32>,
    pub reconnect_attempts: u32,
    pub endpoint_label: Option<String>,
    pub last_disconnect_secs: Option<f32>,
}

/// Check if the app is running in portable mode
pub fn is_portable_mode() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("portable.txt").exists()))
        .unwrap_or(false)
}

/// Get the data directory path (for UI display)
pub fn get_data_directory() -> Option<PathBuf> {
    let exe_path = std::env::current_exe().ok()?;
    let exe_dir = exe_path.parent()?;
    
    // Check for portable marker file
    let marker = exe_dir.join("portable.txt");
    if marker.exists() {
        return Some(exe_dir.join("data"));
    }
    
    // Standard mode
    ProjectDirs::from("com", "hang", "Hang").map(|dirs| dirs.data_dir().to_path_buf())
}

impl SyncClient {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SyncClientState {
                tx: Mutex::new(None),
                room_id: Mutex::new(None),
                client_id: Mutex::new(None),
                is_host: Mutex::new(false),
                session_store: SessionStore::new(),
                stats: Mutex::new(SyncStats::default()),
            }),
        }
    }

    /// Connect to the sync server. Returns a receiver that resolves when the socket closes.
    pub async fn connect<F>(&self, server_url: &str, on_message: F) -> Result<oneshot::Receiver<()>>
    where
        F: Fn(Message) + Send + Sync + 'static,
    {
        let (ws_stream, _) = connect_async(server_url)
            .await
            .context("Failed to connect to server")?;

        let (mut ws_sender, mut ws_receiver) = ws_stream.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<WsMessage>();
        *self.inner.tx.lock() = Some(tx.clone());

        let (disconnect_tx, disconnect_rx) = oneshot::channel();
        let disconnect_signal = Arc::new(Mutex::new(Some(disconnect_tx)));

        // Sender task
        let send_inner = Arc::clone(&self.inner);
        let send_signal = Arc::clone(&disconnect_signal);
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if ws_sender.send(msg).await.is_err() {
                    break;
                }
            }
            send_inner.clear_transport();
            if let Some(tx) = send_signal.lock().take() {
                let _ = tx.send(());
            }
        });

        let handler = Arc::new(on_message);
        let recv_inner = Arc::clone(&self.inner);
        let recv_signal = Arc::clone(&disconnect_signal);
        tokio::spawn(async move {
            while let Some(msg) = ws_receiver.next().await {
                match msg {
                    Ok(WsMessage::Text(text)) => {
                        recv_inner.record_incoming(text.len() as u64);
                        if let Ok(parsed) = serde_json::from_str::<Message>(&text) {
                            handler(parsed);
                        }
                    }
                    Ok(WsMessage::Pong(payload)) => {
                        recv_inner.handle_ws_pong(&payload);
                    }
                    Ok(WsMessage::Close(_)) => break,
                    Err(_) => break,
                    _ => {}
                }
            }
            recv_inner.clear_transport();
            if let Some(tx) = recv_signal.lock().take() {
                let _ = tx.send(());
            }
        });

        // Keep-alive pings
        let ping_inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(12)).await;
                if ping_inner.send_keepalive().is_err() {
                    break;
                }
            }
        });

        Ok(disconnect_rx)
    }

    pub fn mark_connected(&self, label: &str) {
        self.inner.mark_connected(label);
    }

    pub fn mark_disconnected(&self) {
        self.inner.mark_disconnected();
    }

    pub fn stats_snapshot(&self) -> SyncStatsSnapshot {
        self.inner.snapshot()
    }

    /// Create a new room
    pub fn create_room(
        &self,
        file_hash: String,
        passcode: Option<String>,
        display_name: Option<String>,
        capacity: Option<usize>,
    ) -> Result<()> {
        self.send_message(Message::CreateRoom {
            file_hash,
            passcode,
            display_name,
            capacity,
        })
    }

    /// Join an existing room
    pub fn join_room(
        &self,
        room_id: String,
        file_hash: String,
        passcode: Option<String>,
        display_name: Option<String>,
    ) -> Result<()> {
        self.send_message(Message::JoinRoom {
            room_id,
            file_hash,
            passcode,
            display_name,
        })
    }

    /// Leave current room
    pub fn leave_room(&self) -> Result<()> {
        self.send_message(Message::LeaveRoom)
    }

    /// Attempt to resume a room using a server-issued token
    pub fn resume_session(&self, token: String, display_name: Option<String>) -> Result<()> {
        self.send_message(Message::ResumeSession {
            token,
            display_name,
        })
    }

    /// Send a sync command
    pub fn send_sync_command(&self, command: SyncCommand) -> Result<()> {
        self.send_message(Message::SyncCommand(command))
    }

    /// Update room state after receiving server response
    pub fn set_room_joined(&self, room_id: String, client_id: Uuid, is_host: bool) {
        *self.inner.room_id.lock() = Some(room_id);
        *self.inner.client_id.lock() = Some(client_id);
        *self.inner.is_host.lock() = is_host;
    }

    /// Clear room state
    pub fn clear_room(&self) {
        *self.inner.room_id.lock() = None;
        *self.inner.client_id.lock() = None;
        *self.inner.is_host.lock() = false;
    }

    /// Persist the latest successful session locally
    pub fn persist_session(&self, session: &PersistedSession) -> Result<()> {
        self.inner.session_store.save(session)
    }

    /// Remove any locally cached session token
    pub fn clear_persisted_session(&self) -> Result<()> {
        self.inner.session_store.clear();
        Ok(())
    }

    /// Fetch the most recently cached session (if any)
    pub fn saved_session(&self) -> Option<PersistedSession> {
        self.inner.session_store.load()
    }

    fn send_message(&self, msg: Message) -> Result<()> {
        let json = serde_json::to_string(&msg).context("Failed to serialize message")?;
        self.inner.record_outgoing(json.len() as u64);
        if let Some(tx) = self.inner.tx.lock().clone() {
            tx.send(WsMessage::Text(json.into()))
                .context("Failed to queue message to socket")?;
        }
        Ok(())
    }
}

impl SyncClientState {
    fn record_outgoing(&self, bytes: u64) {
        let mut stats = self.stats.lock();
        stats.bytes_out += bytes;
        stats.messages_out += 1;
        stats.last_message_at = Some(Instant::now());
    }

    fn record_incoming(&self, bytes: u64) {
        let mut stats = self.stats.lock();
        stats.bytes_in += bytes;
        stats.messages_in += 1;
        stats.last_message_at = Some(Instant::now());
    }

    fn handle_ws_pong(&self, payload: &[u8]) {
        self.record_incoming(payload.len() as u64);
        if payload.len() < 8 {
            return;
        }
        let mut nonce_bytes = [0u8; 8];
        nonce_bytes.copy_from_slice(&payload[..8]);
        let nonce = u64::from_le_bytes(nonce_bytes);
        self.record_pong(nonce);
    }

    fn record_pong(&self, nonce: u64) {
        let mut stats = self.stats.lock();
        if stats.last_ping_nonce == Some(nonce) {
            if let Some(sent) = stats.last_ping_sent {
                stats.last_rtt_ms = Some(sent.elapsed().as_secs_f32() * 1000.0);
            }
            stats.last_ping_nonce = None;
            stats.last_ping_sent = None;
        }
    }

    fn send_keepalive(&self) -> Result<(), ()> {
        let nonce = Uuid::new_v4().as_u128() as u64;
        {
            let mut stats = self.stats.lock();
            stats.last_ping_nonce = Some(nonce);
            stats.last_ping_sent = Some(Instant::now());
        }

        let mut payload = Vec::with_capacity(24);
        payload.extend_from_slice(&nonce.to_le_bytes());
        payload.extend_from_slice(&current_unix_millis().to_le_bytes());
        self.record_outgoing(payload.len() as u64);
        self.enqueue_ws(WsMessage::Ping(payload.into()))
    }

    fn clear_transport(&self) {
        *self.tx.lock() = None;
        let mut stats = self.stats.lock();
        stats.last_ping_nonce = None;
        stats.last_ping_sent = None;
    }

    fn enqueue_ws(&self, message: WsMessage) -> Result<(), ()> {
        if let Some(tx) = self.tx.lock().clone() {
            tx.send(message).map_err(|_| ())
        } else {
            Err(())
        }
    }

    fn mark_connected(&self, label: &str) {
        let mut stats = self.stats.lock();
        stats.connected_since = Some(Instant::now());
        stats.endpoint_label = Some(label.to_string());
    }

    fn mark_disconnected(&self) {
        let mut stats = self.stats.lock();
        stats.connected_since = None;
        stats.reconnect_attempts += 1;
        stats.last_disconnect_at = Some(Instant::now());
    }

    fn snapshot(&self) -> SyncStatsSnapshot {
        let stats = self.stats.lock();
        let last_message_age = stats
            .last_message_at
            .map(|inst| inst.elapsed().as_secs_f32());
        let connected_duration = stats
            .connected_since
            .map(|inst| inst.elapsed().as_secs_f32());
        let last_disconnect_secs = stats
            .last_disconnect_at
            .map(|inst| inst.elapsed().as_secs_f32());
        SyncStatsSnapshot {
            bytes_out: stats.bytes_out,
            bytes_in: stats.bytes_in,
            messages_out: stats.messages_out,
            messages_in: stats.messages_in,
            last_rtt_ms: stats.last_rtt_ms,
            last_message_age,
            connected_duration,
            reconnect_attempts: stats.reconnect_attempts,
            endpoint_label: stats.endpoint_label.clone(),
            last_disconnect_secs,
        }
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
        // Check for portable mode first
        if let Some(portable_path) = Self::portable_data_path() {
            return portable_path;
        }
        
        // Standard mode: use system directories
        if let Some(dirs) = ProjectDirs::from("com", "hang", "Hang") {
            let data_dir = dirs.data_dir();
            let _ = fs::create_dir_all(data_dir);
            data_dir.join("session.json")
        } else {
            env::temp_dir().join("hang-session.json")
        }
    }
    
    /// Check if running in portable mode (portable.txt exists next to the exe)
    fn portable_data_path() -> Option<PathBuf> {
        let exe_path = env::current_exe().ok()?;
        let exe_dir = exe_path.parent()?;
        
        // Check for portable marker file
        let marker = exe_dir.join("portable.txt");
        if !marker.exists() {
            return None;
        }
        
        // Create data folder next to exe
        let data_dir = exe_dir.join("data");
        let _ = fs::create_dir_all(&data_dir);
        Some(data_dir.join("session.json"))
    }
}

fn current_unix_millis() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|dur| dur.as_millis())
        .unwrap_or(0)
}
