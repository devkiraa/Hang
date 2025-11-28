use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::protocol::{ClientInfo, Room};

/// Shared server state
#[derive(Clone)]
pub struct ServerState {
    /// All active rooms: room_id -> Room
    pub rooms: Arc<DashMap<Uuid, Room>>,
    /// All connected clients: client_id -> ClientInfo
    pub clients: Arc<DashMap<Uuid, ClientInfo>>,
    /// Room membership: room_id -> Vec<client_id>
    pub room_members: Arc<DashMap<Uuid, Arc<RwLock<Vec<Uuid>>>>>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(DashMap::new()),
            clients: Arc::new(DashMap::new()),
            room_members: Arc::new(DashMap::new()),
        }
    }

    pub fn create_room(&self, host_id: Uuid, file_hash: String) -> Uuid {
        let room_id = Uuid::new_v4();
        let room = Room {
            host_id,
            file_hash: file_hash.clone(),
        };

        self.rooms.insert(room_id, room);
        self.room_members
            .insert(room_id, Arc::new(RwLock::new(vec![host_id])));

        // Update client's room
        if let Some(mut client) = self.clients.get_mut(&host_id) {
            client.room_id = Some(room_id);
        }

        tracing::info!("Room {} created by client {}", room_id, host_id);
        room_id
    }

    pub async fn join_room(
        &self,
        client_id: Uuid,
        room_id: Uuid,
        file_hash: &str,
    ) -> Result<bool, String> {
        // Check if room exists
        let room = self
            .rooms
            .get(&room_id)
            .ok_or_else(|| "Room not found".to_string())?;

        // Verify file hash matches
        if room.file_hash != file_hash {
            return Err("File hash mismatch".to_string());
        }

        let is_host = room.host_id == client_id;
        drop(room);

        // Add client to room members
        if let Some(members) = self.room_members.get(&room_id) {
            let mut members = members.write().await;
            if !members.contains(&client_id) {
                members.push(client_id);
            }
        }

        // Update client's room
        if let Some(mut client) = self.clients.get_mut(&client_id) {
            client.room_id = Some(room_id);
        }

        tracing::info!("Client {} joined room {}", client_id, room_id);
        Ok(is_host)
    }

    pub async fn leave_room(&self, client_id: Uuid) {
        // Get client's current room
        let room_id = self.clients.get(&client_id).and_then(|c| c.room_id.clone());

        if let Some(room_id) = room_id {
            // Remove from room members
            if let Some(members) = self.room_members.get(&room_id) {
                let mut members = members.write().await;
                members.retain(|id| *id != client_id);

                // If room is empty, clean it up
                if members.is_empty() {
                    drop(members);
                    self.room_members.remove(&room_id);
                    self.rooms.remove(&room_id);
                    tracing::info!("Room {} deleted (empty)", room_id);
                }
            }

            // Clear client's room
            if let Some(mut client) = self.clients.get_mut(&client_id) {
                client.room_id = None;
            }

            tracing::info!("Client {} left room {}", client_id, room_id);
        }
    }

    pub async fn get_room_members(&self, room_id: Uuid) -> Vec<Uuid> {
        self.room_members
            .get(&room_id)
            .map(|members| {
                let members = members.blocking_read();
                members.clone()
            })
            .unwrap_or_default()
    }

    pub fn add_client(&self, client_id: Uuid) {
        self.clients.insert(client_id, ClientInfo { room_id: None });
        tracing::info!("Client {} connected", client_id);
    }

    pub async fn remove_client(&self, client_id: Uuid) {
        self.leave_room(client_id).await;
        self.clients.remove(&client_id);
        tracing::info!("Client {} disconnected", client_id);
    }
}
