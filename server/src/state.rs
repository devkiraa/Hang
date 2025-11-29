use dashmap::DashMap;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::protocol::{ClientInfo, Room};

const LOG_TAG: &str = "[Hang Server]";

/// Shared server state
#[derive(Clone)]
pub struct ServerState {
    /// All active rooms: room_id -> Room
    pub rooms: Arc<DashMap<String, Room>>,
    /// All connected clients: client_id -> ClientInfo
    pub clients: Arc<DashMap<Uuid, ClientInfo>>,
    /// Room membership: room_id -> Vec<client_id>
    pub room_members: Arc<DashMap<String, Arc<RwLock<Vec<Uuid>>>>>,
    /// Resume tokens issued for reconnect support
    resume_tokens: Arc<DashMap<String, ResumeRecord>>,
    /// Mapping of client id to the last token we issued
    client_tokens: Arc<DashMap<Uuid, String>>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(DashMap::new()),
            clients: Arc::new(DashMap::new()),
            room_members: Arc::new(DashMap::new()),
            resume_tokens: Arc::new(DashMap::new()),
            client_tokens: Arc::new(DashMap::new()),
        }
    }

    pub fn create_room(
        &self,
        host_id: Uuid,
        file_hash: String,
        passcode: Option<String>,
    ) -> (String, bool) {
        let room_id = self.generate_room_code();
        let passcode_hash = passcode
            .filter(|code| !code.is_empty())
            .map(|code| Self::hash_passcode(&code, &room_id));
        let room = Room {
            host_id,
            file_hash: file_hash.clone(),
            passcode_hash: passcode_hash.clone(),
        };

        self.rooms.insert(room_id.clone(), room);
        self.room_members
            .insert(room_id.clone(), Arc::new(RwLock::new(vec![host_id])));

        // Update client's room
        if let Some(mut client) = self.clients.get_mut(&host_id) {
            client.room_id = Some(room_id.clone());
        }

        tracing::info!("{LOG_TAG} Room {} created by client {}", room_id, host_id);
        (room_id, passcode_hash.is_some())
    }

    pub async fn join_room(
        &self,
        client_id: Uuid,
        room_id: &str,
        file_hash: &str,
        passcode: Option<String>,
    ) -> Result<(bool, String), String> {
        // Check if room exists
        let room = self
            .rooms
            .get(room_id)
            .ok_or_else(|| "Room not found".to_string())?;

        // Verify file hash matches
        if room.file_hash != file_hash {
            return Err("File hash mismatch".to_string());
        }

        if let Some(expected) = &room.passcode_hash {
            let provided = passcode
                .as_ref()
                .filter(|code| !code.is_empty())
                .ok_or_else(|| "Passcode required".to_string())?;
            let computed = Self::hash_passcode(provided, room_id);
            if &computed != expected {
                return Err("Invalid passcode".to_string());
            }
        }

        let is_host = room.host_id == client_id;
        let canonical_hash = room.file_hash.clone();
        drop(room);

        // Add client to room members
        if let Some(members) = self.room_members.get(room_id) {
            let mut members = members.write().await;
            if !members.contains(&client_id) {
                members.push(client_id);
            }
        }

        // Update client's room
        if let Some(mut client) = self.clients.get_mut(&client_id) {
            client.room_id = Some(room_id.to_string());
        }

        tracing::info!("{LOG_TAG} Client {} joined room {}", client_id, room_id);
        Ok((is_host, canonical_hash))
    }

    pub async fn leave_room(&self, client_id: Uuid) -> Option<String> {
        // Get client's current room
        let room_id = self.clients.get(&client_id).and_then(|c| c.room_id.clone());

        if let Some(room_id) = room_id.clone() {
            // Remove from room members
            if let Some(members) = self.room_members.get(&room_id) {
                let mut members = members.write().await;
                members.retain(|id| *id != client_id);

                // If room is empty, clean it up
                if members.is_empty() {
                    drop(members);
                    self.room_members.remove(&room_id);
                    self.rooms.remove(&room_id);
                    self.clear_tokens_for_room(&room_id);
                    tracing::info!("{LOG_TAG} Room {} deleted (empty)", room_id);
                    return Some(room_id);
                }
            }

            // Clear client's room
            if let Some(mut client) = self.clients.get_mut(&client_id) {
                client.room_id = None;
            }

            tracing::info!("{LOG_TAG} Client {} left room {}", client_id, room_id);
            Some(room_id)
        } else {
            None
        }
    }

    pub async fn get_room_members(&self, room_id: &str) -> Vec<Uuid> {
        if let Some(members_ref) = self.room_members.get(room_id) {
            let members_lock = Arc::clone(&*members_ref);
            drop(members_ref);
            let members = members_lock.read().await;
            members.clone()
        } else {
            Vec::new()
        }
    }

    pub fn add_client(&self, client_id: Uuid) {
        self.clients.insert(client_id, ClientInfo { room_id: None });
        tracing::info!("{LOG_TAG} Client {} connected", client_id);
    }

    pub async fn remove_client(&self, client_id: Uuid) {
        let _ = self.leave_room(client_id).await;
        self.clients.remove(&client_id);
        tracing::info!("{LOG_TAG} Client {} disconnected", client_id);
    }

    pub fn remember_session(
        &self,
        client_id: Uuid,
        room_id: &str,
        file_hash: &str,
        was_host: bool,
    ) -> String {
        let token = Uuid::new_v4().to_string();
        if let Some(previous) = self.client_tokens.insert(client_id, token.clone()) {
            self.resume_tokens.remove(&previous);
        }

        self.resume_tokens.insert(
            token.clone(),
            ResumeRecord {
                client_id,
                room_id: room_id.to_string(),
                file_hash: file_hash.to_string(),
                was_host,
            },
        );

        token
    }

    pub fn clear_session(&self, client_id: Uuid) {
        if let Some((_, token)) = self.client_tokens.remove(&client_id) {
            self.resume_tokens.remove(&token);
        }
    }

    fn clear_tokens_for_room(&self, room_id: &str) {
        let tokens: Vec<String> = self
            .resume_tokens
            .iter()
            .filter_map(|entry| {
                if entry.value().room_id == room_id {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        for token in tokens {
            if let Some((_, record)) = self.resume_tokens.remove(&token) {
                self.client_tokens.remove(&record.client_id);
            }
        }
    }

    pub async fn resume_session(
        &self,
        client_id: Uuid,
        token: &str,
    ) -> Result<ResumeOutcome, String> {
        let record = self
            .resume_tokens
            .remove(token)
            .map(|(_, rec)| rec)
            .ok_or_else(|| "Session token invalid or expired".to_string())?;
        self.client_tokens.remove(&record.client_id);

        let passcode_enabled = self
            .rooms
            .get(&record.room_id)
            .map(|room| room.passcode_hash.is_some())
            .ok_or_else(|| "Room not found".to_string())?;

        if record.was_host {
            if let Some(mut room) = self.rooms.get_mut(&record.room_id) {
                room.host_id = client_id;
            }
        }

        if let Some(members) = self.room_members.get(&record.room_id) {
            let mut members = members.write().await;
            if !members.contains(&client_id) {
                members.push(client_id);
            }
        } else {
            return Err("Room is no longer active".to_string());
        }

        self.clients.insert(
            client_id,
            ClientInfo {
                room_id: Some(record.room_id.clone()),
            },
        );

        let new_token = self.remember_session(
            client_id,
            &record.room_id,
            &record.file_hash,
            record.was_host,
        );

        Ok(ResumeOutcome {
            room_id: record.room_id,
            was_host: record.was_host,
            passcode_enabled,
            resume_token: new_token,
            file_hash: record.file_hash,
        })
    }

    fn generate_room_code(&self) -> String {
        loop {
            let raw = (Uuid::new_v4().as_u128() % 1_000_000) as u32;
            let code = format!("{:03}-{:03}", raw / 1000, raw % 1000);
            if !self.rooms.contains_key(&code) {
                break code;
            }
        }
    }

    fn hash_passcode(passcode: &str, room_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(room_id.as_bytes());
        hasher.update(passcode.as_bytes());
        let digest = hasher.finalize();
        format!("{:x}", digest)
    }
}

#[derive(Clone)]
pub struct ResumeRecord {
    pub client_id: Uuid,
    pub room_id: String,
    pub file_hash: String,
    pub was_host: bool,
}

pub struct ResumeOutcome {
    pub room_id: String,
    pub was_host: bool,
    pub passcode_enabled: bool,
    pub resume_token: String,
    pub file_hash: String,
}
