use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use uuid::Uuid;

use crate::protocol::{Message, SyncCommand};

pub struct SyncClient {
    tx: Mutex<Option<mpsc::UnboundedSender<Message>>>,
    room_id: Mutex<Option<Uuid>>,
    client_id: Mutex<Option<Uuid>>,
    is_host: Mutex<bool>,
}

impl SyncClient {
    pub fn new() -> Self {
        Self {
            tx: Mutex::new(None),
            room_id: Mutex::new(None),
            client_id: Mutex::new(None),
            is_host: Mutex::new(false),
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
    pub fn create_room(&self, file_hash: String) -> Result<()> {
        self.send_message(Message::CreateRoom { file_hash })
    }

    /// Join an existing room
    pub fn join_room(&self, room_id: Uuid, file_hash: String) -> Result<()> {
        self.send_message(Message::JoinRoom { room_id, file_hash })
    }

    /// Leave current room
    pub fn leave_room(&self) -> Result<()> {
        self.send_message(Message::LeaveRoom)
    }

    /// Send a sync command
    pub fn send_sync_command(&self, command: SyncCommand) -> Result<()> {
        self.send_message(Message::SyncCommand(command))
    }

    /// Update room state after receiving server response
    pub fn set_room_joined(&self, room_id: Uuid, client_id: Uuid, is_host: bool) {
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

    fn send_message(&self, msg: Message) -> Result<()> {
        if let Some(tx) = self.tx.lock().as_ref() {
            tx.send(msg).context("Failed to send message")?;
        }
        Ok(())
    }
}
