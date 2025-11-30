use eframe::egui;
use parking_lot::Mutex;
use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::mpsc::{error::TryRecvError, UnboundedReceiver, UnboundedSender};

use crate::{
    constants::VERSION,
    invite::{self, InviteLink, InviteSignal},
    player::{VideoFrame, VideoPlayer},
    protocol::{MemberSummary, Message, SyncCommand},
    sync::{get_data_directory, is_portable_mode, PersistedSession, SyncClient, SyncStatsSnapshot},
    update::{self, UpdateInfo},
    utils::{compute_file_hash, format_time},
    youtube,
};
use uuid::Uuid;

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v"];
const KEYBOARD_SEEK_STEP: f64 = 5.0;
const KEYBOARD_VOLUME_STEP: f64 = 5.0;
const ROOM_CAPACITY_MIN: u32 = 2;
const ROOM_CAPACITY_MAX: u32 = 32;
const DEFAULT_ROOM_CAPACITY: u32 = 12;

pub struct HangApp {
    // Video player
    player: Arc<VideoPlayer>,

    // Sync client
    sync: Arc<SyncClient>,

    // UI state
    video_file: Option<PathBuf>,
    video_hash: Option<String>,
    room_id_input: String,
    create_passcode_input: String,
    join_passcode_input: String,
    display_name_input: String,
    status_message: String,
    error_message: Option<String>,

    // Playback state
    is_playing: bool,
    current_position: f64,
    duration: f64,
    volume: f64,
    speed: f64,

    // Room state
    in_room: bool,
    current_room_id: Option<String>,
    is_host: bool,
    participant_count: usize,
    member_roster: Vec<MemberSummary>,
    room_dialog_open: bool,
    is_fullscreen: bool,
    controls_visible: bool,
    active_room_passcode: Option<String>,
    pending_room_passcode: Option<String>,
    room_has_passcode: bool,
    room_capacity_input: u32,
    room_capacity_limit: Option<usize>,

    // Settings panel
    show_settings: bool,
    audio_tracks: Vec<crate::player::AudioTrack>,
    subtitle_tracks: Vec<crate::player::SubtitleTrack>,
    selected_audio: i64,
    selected_subtitle: i64,

    // Sync control
    sync_enabled: bool,
    sync_connected: bool,
    last_sync_time: Arc<Mutex<std::time::Instant>>,
    invite_rx: Option<UnboundedReceiver<InviteSignal>>,
    pending_invite: Option<InviteLink>,
    invite_modal_open: bool,
    sync_reconnect_tx: Option<UnboundedSender<()>>,
    saved_session: Option<PersistedSession>,
    auto_resume_attempted: bool,
    resume_in_progress: bool,
    show_about: bool,
    show_network_overlay: bool,

    // Update state
    update_info: Option<UpdateInfo>,
    update_check_done: bool,
    show_url_dialog: bool,
    url_input: String,
    youtube_quality: youtube::VideoQuality,
    youtube_loader: Option<youtube::YouTubeLoader>,
    youtube_loading_url: Option<String>,
    current_youtube_url: Option<String>,  // Store current YouTube URL for quality changes
    pending_youtube_seek_position: Option<f64>,  // Seek position to restore after quality change
    
    // Buffering state
    is_buffering: bool,
    last_frame_time: std::time::Instant,
    buffering_start_time: Option<std::time::Instant>,

    // Video rendering
    video_texture: Option<egui::TextureHandle>,
    last_frame_size: Option<(u32, u32)>,
}

impl HangApp {
    pub fn new(
        _cc: &eframe::CreationContext,
        player: Arc<VideoPlayer>,
        sync: Arc<SyncClient>,
        invite_rx: UnboundedReceiver<InviteSignal>,
        sync_reconnect_tx: UnboundedSender<()>,
    ) -> Self {
        let cached_session = sync.saved_session();
        Self {
            player,
            sync,
            video_file: None,
            video_hash: None,
            room_id_input: String::new(),
            create_passcode_input: String::new(),
            join_passcode_input: String::new(),
            display_name_input: Self::default_display_name(),
            status_message: "Connecting to sync server (may take up to a minute)...".to_string(),
            error_message: None,
            is_playing: false,
            current_position: 0.0,
            duration: 0.0,
            volume: 100.0,
            speed: 1.0,
            in_room: false,
            current_room_id: None,
            is_host: false,
            participant_count: 0,
            member_roster: Vec::new(),
            room_dialog_open: false,
            is_fullscreen: false,
            controls_visible: true,
            active_room_passcode: None,
            pending_room_passcode: None,
            room_has_passcode: false,
            room_capacity_input: DEFAULT_ROOM_CAPACITY,
            room_capacity_limit: None,
            show_settings: false,
            audio_tracks: Vec::new(),
            subtitle_tracks: Vec::new(),
            selected_audio: -1,
            selected_subtitle: -1,
            sync_enabled: true,
            sync_connected: false,
            last_sync_time: Arc::new(Mutex::new(std::time::Instant::now())),
            invite_rx: Some(invite_rx),
            pending_invite: None,
            invite_modal_open: false,
            sync_reconnect_tx: Some(sync_reconnect_tx),
            saved_session: cached_session,
            auto_resume_attempted: false,
            resume_in_progress: false,
            show_about: false,
            show_network_overlay: false,
            update_info: None,
            update_check_done: false,
            show_url_dialog: false,
            url_input: String::new(),
            youtube_quality: youtube::VideoQuality::default(),
            youtube_loader: None,
            youtube_loading_url: None,
            current_youtube_url: None,
            pending_youtube_seek_position: None,
            is_buffering: false,
            last_frame_time: std::time::Instant::now(),
            buffering_start_time: None,
            video_texture: None,
            last_frame_size: None,
        }
    }

    pub fn update_sync_status<S: Into<String>>(&mut self, message: S, connected: Option<bool>) {
        self.status_message = message.into();
        if let Some(flag) = connected {
            self.sync_connected = flag;
        }
    }
    
    /// Clean up resources before app exit for smooth shutdown
    pub fn cleanup(&mut self) {
        tracing::info!("Cleaning up before exit...");
        
        // Stop any playing video
        if self.is_playing {
            let _ = self.player.stop();
            self.is_playing = false;
        }
        
        // Cancel any pending YouTube loader
        self.youtube_loader = None;
        self.youtube_loading_url = None;
        
        // Clear video texture
        self.video_texture = None;
        
        // Leave room if connected
        if self.in_room {
            let _ = self.sync.leave_room();
        }
        
        // Small delay to let cleanup complete
        std::thread::sleep(std::time::Duration::from_millis(50));
        
        tracing::info!("Cleanup complete");
    }

    fn load_video_from_path(&mut self, path: &Path) -> Result<(), String> {
        self.player.load_file(path)?;
        let hash = compute_file_hash(path).map_err(|e| e.to_string())?;

        self.video_file = Some(path.to_path_buf());
        self.video_hash = Some(hash);
        self.video_texture = None;
        self.last_frame_size = None;
        self.status_message = format!(
            "Loaded: {}",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
        );
        self.error_message = None;

        if let Err(e) = self.refresh_media_tracks() {
            self.error_message = Some(e);
        }

        if let Err(e) = self.player.play() {
            self.error_message = Some(format!("Failed to auto-play: {}", e));
        } else {
            self.is_playing = true;
        }

        Ok(())
    }

    fn is_supported_video(path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                VIDEO_EXTENSIONS
                    .iter()
                    .any(|allowed| ext.eq_ignore_ascii_case(allowed))
            })
            .unwrap_or(false)
    }

    fn normalize_passcode(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn default_display_name() -> String {
        env::var("USERNAME")
            .ok()
            .or_else(|| env::var("USER").ok())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| format!("Guest {}", &Uuid::new_v4().to_string()[..4]))
    }

    fn sanitized_display_name(&self) -> Option<String> {
        Self::sanitize_display_name_str(&self.display_name_input)
    }

    fn sanitize_display_name_str(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }
        let mut cleaned = String::with_capacity(trimmed.len().min(32));
        for ch in trimmed.chars() {
            if ch.is_control() {
                continue;
            }
            if cleaned.len() >= 32 {
                break;
            }
            cleaned.push(ch);
        }
        if cleaned.is_empty() {
            None
        } else {
            Some(cleaned)
        }
    }

    fn refresh_media_tracks(&mut self) -> Result<(), String> {
        self.audio_tracks = self.player.get_audio_tracks()?;
        if !self
            .audio_tracks
            .iter()
            .any(|track| track.id == self.selected_audio)
        {
            self.selected_audio = self
                .audio_tracks
                .first()
                .map(|track| track.id)
                .unwrap_or(-1);
        }

        self.subtitle_tracks = self.player.get_subtitle_tracks()?;
        if self.selected_subtitle != -1
            && !self
                .subtitle_tracks
                .iter()
                .any(|track| track.id == self.selected_subtitle)
        {
            self.selected_subtitle = -1;
        }

        Ok(())
    }

    fn apply_member_roster(&mut self, roster: Vec<MemberSummary>, capacity: usize) {
        self.participant_count = roster.len().max(1);
        self.member_roster = roster;
        self.room_capacity_limit = Some(capacity);
        self.room_capacity_input = capacity as u32;
    }

    fn format_bytes_short(bytes: u64) -> String {
        const KB: f64 = 1024.0;
        const MB: f64 = KB * 1024.0;
        const GB: f64 = MB * 1024.0;
        let value = bytes as f64;
        if value < KB {
            format!("{} B", bytes)
        } else if value < MB {
            format!("{:.1} KB", value / KB)
        } else if value < GB {
            format!("{:.1} MB", value / MB)
        } else {
            format!("{:.2} GB", value / GB)
        }
    }

    fn describe_duration(secs: Option<f32>) -> String {
        secs.map(|value| {
            if value.is_nan() {
                "—".to_string()
            } else if value >= 120.0 {
                format!("{:.1} min", value / 60.0)
            } else {
                format!("{:.1} s", value)
            }
        })
        .unwrap_or_else(|| "—".to_string())
    }

    fn update_playback_state(&mut self) {
        if let Ok(pos) = self.player.get_position() {
            self.current_position = pos;
        }
        if let Ok(dur) = self.player.get_duration() {
            if dur > 0.0 {
                self.duration = dur;
            }
        }
        if let Ok(paused) = self.player.is_paused() {
            self.is_playing = !paused;
        }
        if let Ok(vol) = self.player.get_volume() {
            self.volume = vol;
        }
        if let Ok(spd) = self.player.get_speed() {
            self.speed = spd;
        }
    }

    fn poll_invite_channel(&mut self) {
        loop {
            let result = {
                let Some(rx) = self.invite_rx.as_mut() else {
                    return;
                };
                match rx.try_recv() {
                    Ok(signal) => Ok(Some(signal)),
                    Err(TryRecvError::Empty) => Ok(None),
                    Err(TryRecvError::Disconnected) => Err(()),
                }
            };

            match result {
                Ok(Some(signal)) => self.process_invite_signal(signal),
                Ok(None) => break,
                Err(()) => {
                    self.invite_rx = None;
                    break;
                }
            }
        }
    }

    fn request_manual_reconnect(&mut self) {
        if let Some(tx) = self.sync_reconnect_tx.as_ref() {
            let _ = tx.send(());
            self.sync_connected = false;
            self.status_message = "Retrying sync connection...".into();
        }
    }

    fn attempt_resume(&mut self, automatic: bool) {
        let Some(session) = self.saved_session.clone() else {
            return;
        };
        if automatic && self.auto_resume_attempted {
            return;
        }
        if !self.sync_connected {
            if !automatic {
                self.error_message =
                    Some("Cannot resume until the sync server connection is ready".into());
            }
            return;
        }
        match self
            .sync
            .resume_session(session.resume_token.clone(), self.sanitized_display_name())
        {
            Ok(_) => {
                self.status_message = format!("Attempting to resume room {}...", session.room_id);
                self.resume_in_progress = true;
                if automatic {
                    self.auto_resume_attempted = true;
                }
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to resume session: {}", e));
            }
        }
    }

    fn maybe_auto_resume(&mut self) {
        if self.in_room || self.resume_in_progress {
            return;
        }
        if self.saved_session.is_none() || !self.sync_connected {
            return;
        }
        self.attempt_resume(true);
    }

    fn remember_session(
        &mut self,
        room_id: String,
        resume_token: String,
        file_hash: String,
        is_host: bool,
    ) {
        let session = PersistedSession {
            room_id,
            resume_token,
            file_hash,
            is_host,
        };
        if let Err(e) = self.sync.persist_session(&session) {
            self.error_message = Some(format!("Failed to cache session: {}", e));
        }
        self.saved_session = Some(session);
        self.auto_resume_attempted = false;
        self.resume_in_progress = false;
    }

    fn clear_saved_session(&mut self) {
        let _ = self.sync.clear_persisted_session();
        self.saved_session = None;
        self.auto_resume_attempted = false;
        self.resume_in_progress = false;
    }

    pub fn handle_connection_loss(&mut self) {
        self.sync_connected = false;
        self.in_room = false;
        self.current_room_id = None;
        self.is_host = false;
        self.participant_count = 0;
        self.member_roster.clear();
        self.room_capacity_limit = None;
        self.status_message = "Sync connection lost. Reconnecting...".to_string();
        self.resume_in_progress = false;
    }

    fn process_invite_signal(&mut self, signal: InviteSignal) {
        match invite::parse_invite_url(&signal.url) {
            Some(link) => {
                self.room_id_input = link.room_id.clone();
                self.sanitize_room_code_input();
                if let Some(passcode) = &link.passcode {
                    self.join_passcode_input = passcode.clone();
                } else {
                    self.join_passcode_input.clear();
                }
                self.pending_invite = Some(link);
                self.invite_modal_open = true;
                self.room_dialog_open = true;
                self.status_message = "Invite received".to_string();
            }
            None => {
                self.error_message = Some("Invalid invite link".to_string());
            }
        }
    }

    fn select_video_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Video Files", VIDEO_EXTENSIONS)
            .pick_file()
        {
            if let Err(e) = self.load_video_from_path(&path) {
                self.error_message = Some(format!("Failed to load video: {}", e));
            }
        }
    }
    
    fn load_video_from_url(&mut self) {
        let url = self.url_input.trim().to_string();
        if url.is_empty() {
            return;
        }
        
        // Validate URL
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.error_message = Some("URL must start with http:// or https://".into());
            return;
        }
        
        // Check if this is a YouTube URL
        if youtube::is_youtube_url(&url) {
            self.load_youtube_video(&url);
            return;
        }
        
        // Try to load the URL directly in VLC
        match self.player.load_url(&url) {
            Ok(()) => {
                // Use URL for hash computation
                let hash = crate::utils::compute_string_hash(&url);
                
                self.video_file = Some(PathBuf::from(&url));
                self.video_hash = Some(hash);
                self.video_texture = None;
                self.last_frame_size = None;
                self.status_message = format!("Loading URL: {}", &url[..url.len().min(50)]);
                self.error_message = None;
                self.show_url_dialog = false;
                self.url_input.clear();
                
                // Auto-play
                if let Err(e) = self.player.play() {
                    self.error_message = Some(format!("Failed to auto-play: {}", e));
                } else {
                    self.is_playing = true;
                }
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to load URL: {}", e));
            }
        }
    }
    
    fn load_youtube_video(&mut self, url: &str) {
        // Start async loading - won't block UI
        self.youtube_loading_url = Some(url.to_string());
        self.youtube_loader = Some(youtube::YouTubeLoader::start(
            url.to_string(),
            self.youtube_quality,
        ));
        self.status_message = if youtube::is_ytdlp_available() {
            "Loading YouTube video...".into()
        } else {
            "Downloading yt-dlp (first time only)...".into()
        };
        self.show_url_dialog = false;
        self.url_input.clear();
    }
    
    fn load_youtube_video_at_position(&mut self, url: &str, seek_position: f64) {
        // Start async loading for quality change - will seek after load
        self.youtube_loading_url = Some(url.to_string());
        self.pending_youtube_seek_position = Some(seek_position);  // Store position to restore
        self.youtube_loader = Some(youtube::YouTubeLoader::start(
            url.to_string(),
            self.youtube_quality,
        ));
        self.status_message = format!("Changing quality to {}...", self.youtube_quality.as_str());
        self.is_buffering = true;
    }
    
    fn poll_youtube_loader(&mut self) {
        if let Some(ref loader) = self.youtube_loader {
            if let Some(result) = loader.try_recv() {
                match result {
                    youtube::YouTubeLoadResult::Downloading => {
                        self.status_message = "Downloading yt-dlp (first time only, ~10MB)...".into();
                    }
                    youtube::YouTubeLoadResult::Success(video_info) => {
                        // Extract video ID for consistent hashing
                        let url = self.youtube_loading_url.take().unwrap_or_default();
                        let video_id = youtube::extract_video_id(&url).unwrap_or_else(|| url.clone());
                        let hash = crate::utils::compute_string_hash(&video_id);
                        
                        // Check if we need to restore a seek position (quality change)
                        let pending_seek = self.pending_youtube_seek_position.take();
                        
                        // Load the direct stream URL
                        match self.player.load_url(&video_info.stream_url) {
                            Ok(()) => {
                                self.video_file = Some(PathBuf::from(format!("youtube://{}", video_id)));
                                self.video_hash = Some(hash);
                                self.video_texture = None;
                                self.last_frame_size = None;
                                self.current_youtube_url = Some(url);  // Save URL for quality changes
                                self.status_message = format!("Playing: {} ({})", video_info.title, video_info.quality.as_str());
                                self.error_message = None;
                                
                                // Auto-play
                                if let Err(e) = self.player.play() {
                                    self.error_message = Some(format!("Failed to auto-play: {}", e));
                                } else {
                                    self.is_playing = true;
                                    
                                    // Restore position if this was a quality change
                                    if let Some(position) = pending_seek {
                                        // Short delay to let player initialize
                                        std::thread::sleep(std::time::Duration::from_millis(100));
                                        if let Err(e) = self.player.seek(position) {
                                            tracing::warn!("Failed to restore position after quality change: {}", e);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                self.error_message = Some(format!("Failed to load stream: {}", e));
                            }
                        }
                        self.youtube_loader = None;
                    }
                    youtube::YouTubeLoadResult::Error(e) => {
                        self.error_message = Some(format!("YouTube error: {}", e));
                        self.youtube_loader = None;
                        self.youtube_loading_url = None;
                        self.current_youtube_url = None;
                    }
                }
            }
        }
    }
    
    fn check_for_updates(&mut self) {
        self.update_check_done = true;
        self.status_message = "Checking for updates...".into();
        
        // Spawn a thread to check for updates
        // Note: In a real app, we'd want to communicate results back to the UI
        // For now, we just log the result
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new();
            if let Ok(rt) = rt {
                match rt.block_on(update::check_for_updates()) {
                    Ok(info) => {
                        tracing::info!(
                            "Update check: current={}, latest={}, available={}",
                            info.current_version,
                            info.latest_version,
                            info.is_update_available
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to check for updates: {}", e);
                    }
                }
            }
        });
    }

    fn create_room(&mut self) {
        if !self.sync_connected {
            self.error_message = Some(
                "Still connecting to the sync server. Please wait for the warm-up and try again."
                    .into(),
            );
            return;
        }
        if let Some(hash) = &self.video_hash {
            let passcode = Self::normalize_passcode(&self.create_passcode_input);
            self.pending_room_passcode = passcode.clone();
            let display_name = self.sanitized_display_name();
            let capacity = Some(self.room_capacity_input as usize);
            if let Err(e) = self
                .sync
                .create_room(hash.clone(), passcode, display_name, capacity)
            {
                self.error_message = Some(format!("Failed to create room: {}", e));
            } else {
                self.status_message = "Creating room...".to_string();
            }
        }
    }

    fn join_room(&mut self) {
        if !self.sync_connected {
            self.error_message = Some(
                "Cannot join until the sync server connection is ready. Render cold-starts may take ~1 minute."
                    .into(),
            );
            return;
        }
        let code = self.room_id_input.trim().to_string();
        if self.video_hash.is_none() {
            self.error_message = Some("Load the same video before joining a room".into());
            return;
        }

        if !Self::is_valid_room_code(&code) {
            self.error_message = Some("Room code must look like 123-456".into());
            return;
        }

        if let Some(hash) = &self.video_hash {
            let passcode = Self::normalize_passcode(&self.join_passcode_input);
            self.pending_room_passcode = passcode.clone();
            let display_name = self.sanitized_display_name();
            if let Err(e) =
                self.sync
                    .join_room(code.clone(), hash.clone(), passcode.clone(), display_name)
            {
                self.error_message = Some(format!("Failed to join room: {}", e));
            } else {
                self.status_message = format!("Joining room {}...", code);
            }
        }
    }

    fn leave_room(&mut self) {
        if let Err(e) = self.sync.leave_room() {
            self.error_message = Some(format!("Failed to leave room: {}", e));
        }
        self.in_room = false;
        self.current_room_id = None;
        self.is_host = false;
        self.participant_count = 0;
        self.member_roster.clear();
        self.room_capacity_limit = None;
        self.status_message = "Left room".to_string();
    }

    fn toggle_play(&mut self) {
        let result = if self.is_playing {
            self.player.pause()
        } else {
            self.player.play()
        };

        if let Err(e) = result {
            self.error_message = Some(format!("Playback error: {}", e));
        } else if self.sync_enabled && self.in_room {
            let _ = self.sync.send_sync_command(if self.is_playing {
                SyncCommand::Pause {
                    timestamp: self.current_position,
                }
            } else {
                SyncCommand::Play {
                    timestamp: self.current_position,
                }
            });
        }
    }

    fn seek(&mut self, position: f64) {
        // Start buffering indicator for YouTube/URL videos
        if self.is_youtube_video() || self.is_url_video() {
            self.is_buffering = true;
            self.buffering_start_time = Some(std::time::Instant::now());
        }
        
        if let Err(e) = self.player.seek(position) {
            self.error_message = Some(format!("Seek error: {}", e));
            self.is_buffering = false;
        } else if self.sync_enabled && self.in_room {
            let _ = self.sync.send_sync_command(SyncCommand::Seek {
                timestamp: position,
            });
        }
    }
    
    fn is_youtube_video(&self) -> bool {
        self.video_file.as_ref()
            .map(|p| p.to_string_lossy().starts_with("youtube://"))
            .unwrap_or(false)
    }
    
    fn is_url_video(&self) -> bool {
        self.video_file.as_ref()
            .map(|p| {
                let s = p.to_string_lossy();
                s.starts_with("http://") || s.starts_with("https://")
            })
            .unwrap_or(false)
    }

    fn set_volume(&mut self, volume: f64) {
        if let Err(e) = self.player.set_volume(volume) {
            self.error_message = Some(format!("Volume error: {}", e));
        }
    }

    fn set_speed(&mut self, speed: f64) {
        if let Err(e) = self.player.set_speed(speed) {
            self.error_message = Some(format!("Speed error: {}", e));
        } else if self.sync_enabled && self.in_room {
            let _ = self
                .sync
                .send_sync_command(SyncCommand::Speed { rate: speed });
        }
    }

    pub fn handle_server_message(&mut self, msg: Message) {
        match msg {
            Message::RoomCreated {
                room_id,
                client_id,
                passcode_enabled,
                file_hash,
                resume_token,
                capacity,
                display_name,
            } => {
                self.sync.set_room_joined(room_id.clone(), client_id, true);
                self.in_room = true;
                self.current_room_id = Some(room_id.clone());
                self.is_host = true;
                self.participant_count = 1;
                self.status_message = format!("Room created: {}", room_id);
                self.room_id_input = room_id.clone();
                self.invite_modal_open = false;
                self.pending_invite = None;
                self.room_has_passcode = passcode_enabled;
                self.room_capacity_limit = Some(capacity);
                self.room_capacity_input = capacity as u32;
                self.active_room_passcode = if passcode_enabled {
                    self.pending_room_passcode.clone()
                } else {
                    None
                };
                self.pending_room_passcode = None;
                if passcode_enabled {
                    self.create_passcode_input.clear();
                }
                self.display_name_input = display_name.clone();
                self.member_roster = vec![MemberSummary {
                    client_id,
                    display_name,
                    is_host: true,
                }];
                self.remember_session(room_id.clone(), resume_token, file_hash, true);
            }
            Message::RoomJoined {
                room_id,
                client_id,
                is_host,
                passcode_enabled,
                file_hash,
                resume_token,
                capacity,
                display_name,
            } => {
                self.sync
                    .set_room_joined(room_id.clone(), client_id, is_host);
                self.in_room = true;
                self.current_room_id = Some(room_id.clone());
                self.is_host = is_host;
                self.participant_count = 1;
                self.status_message = format!(
                    "Joined room: {} ({})",
                    room_id,
                    if is_host { "Host" } else { "Guest" }
                );
                self.invite_modal_open = false;
                self.pending_invite = None;
                self.room_has_passcode = passcode_enabled;
                self.room_capacity_limit = Some(capacity);
                self.room_capacity_input = capacity as u32;
                self.active_room_passcode = if passcode_enabled {
                    self.pending_room_passcode.clone()
                } else {
                    None
                };
                self.pending_room_passcode = None;
                if !is_host {
                    self.join_passcode_input.clear();
                }
                self.display_name_input = display_name.clone();
                self.member_roster = vec![MemberSummary {
                    client_id,
                    display_name,
                    is_host,
                }];
                self.remember_session(room_id, resume_token, file_hash, is_host);
            }
            Message::RoomLeft => {
                self.sync.clear_room();
                self.in_room = false;
                self.current_room_id = None;
                self.is_host = false;
                self.participant_count = 0;
                self.status_message = "Left room".to_string();
                self.room_has_passcode = false;
                self.active_room_passcode = None;
                self.pending_room_passcode = None;
                self.pending_invite = None;
                self.invite_modal_open = false;
                self.member_roster.clear();
                self.room_capacity_limit = None;
                self.clear_saved_session();
            }
            Message::RoomNotFound => {
                self.resume_in_progress = false;
                self.error_message = Some("Room not found".to_string());
            }
            Message::RoomFull { capacity } => {
                self.resume_in_progress = false;
                self.error_message = Some(format!("Room is full ({} seats)", capacity));
            }
            Message::FileHashMismatch { expected } => {
                self.resume_in_progress = false;
                self.error_message =
                    Some(format!("File mismatch! Expected hash: {}", &expected[..16]));
            }
            Message::SyncBroadcast { command, .. } => {
                if self.sync_enabled {
                    self.handle_sync_command(command);
                }
            }
            Message::Error { message } => {
                if message.contains("Session token") {
                    self.clear_saved_session();
                }
                self.resume_in_progress = false;
                self.error_message = Some(message);
            }
            Message::RoomMemberUpdate {
                room_id,
                members,
                capacity,
            } => {
                if self.current_room_id.as_deref() == Some(room_id.as_str()) {
                    self.apply_member_roster(members, capacity);
                }
            }
            _ => {}
        }
    }

    fn handle_sync_command(&mut self, command: SyncCommand) {
        // Debounce rapid sync commands
        let now = std::time::Instant::now();
        let mut last_sync = self.last_sync_time.lock();
        if now.duration_since(*last_sync).as_millis() < 100 {
            return;
        }
        *last_sync = now;
        drop(last_sync);

        match command {
            SyncCommand::Play { timestamp } => {
                let _ = self.player.seek(timestamp);
                let _ = self.player.play();
            }
            SyncCommand::Pause { timestamp } => {
                let _ = self.player.seek(timestamp);
                let _ = self.player.pause();
            }
            SyncCommand::Seek { timestamp } => {
                let _ = self.player.seek(timestamp);
            }
            SyncCommand::Speed { rate } => {
                let _ = self.player.set_speed(rate);
            }
            SyncCommand::Stop => {
                let _ = self.player.stop();
            }
        }
    }

    fn update_video_texture(&mut self, ctx: &egui::Context) {
        if let Some(frame) = self.player.latest_frame() {
            if let Some(image) = Self::frame_to_color_image(&frame) {
                if let Some(texture) = self.video_texture.as_mut() {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.video_texture = Some(ctx.load_texture(
                        "hang-video-frame",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
                self.last_frame_size = Some((frame.width, frame.height));
                
                // New frame received - stop buffering indicator
                self.last_frame_time = std::time::Instant::now();
                if self.is_buffering {
                    self.is_buffering = false;
                    self.buffering_start_time = None;
                }
            }
        }
        
        // Detect stalled playback (no new frames for 500ms while playing)
        if self.is_playing && (self.is_youtube_video() || self.is_url_video()) {
            let elapsed = self.last_frame_time.elapsed();
            if elapsed.as_millis() > 500 && !self.is_buffering {
                self.is_buffering = true;
                self.buffering_start_time = Some(std::time::Instant::now());
            }
        }
    }

    fn frame_to_color_image(frame: &VideoFrame) -> Option<egui::ColorImage> {
        let width = frame.width as usize;
        let height = frame.height as usize;
        if width == 0 || height == 0 {
            return None;
        }
        if frame.buffer.len() < width * height * 4 {
            return None;
        }

        let mut image = egui::ColorImage::new([width, height], egui::Color32::BLACK);
        for (pixel, chunk) in image.pixels.iter_mut().zip(frame.buffer.chunks_exact(4)) {
            *pixel = egui::Color32::from_rgba_unmultiplied(chunk[2], chunk[1], chunk[0], 255);
        }
        Some(image)
    }

    fn fitted_video_size(&self, available: egui::Vec2) -> egui::Vec2 {
        let aspect = self
            .last_frame_size
            .map(|(w, h)| w as f32 / (h.max(1) as f32))
            .unwrap_or(16.0 / 9.0);
        let mut size = available;
        if size.x <= 0.0 || size.y <= 0.0 {
            size = egui::vec2(1.0, 1.0);
        }
        let current_aspect = size.x / size.y;
        if current_aspect > aspect {
            size.x = size.y * aspect;
        } else {
            size.y = size.x / aspect;
        }
        size.x = size.x.max(1.0);
        size.y = size.y.max(1.0);
        size
    }

    fn handle_file_drop(&mut self, ctx: &egui::Context) {
        let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped_files.is_empty() {
            return;
        }

        for file in dropped_files {
            if let Some(path) = file.path {
                if !Self::is_supported_video(&path) {
                    self.error_message = Some("Unsupported file type".into());
                    continue;
                }
                if let Err(e) = self.load_video_from_path(&path) {
                    self.error_message = Some(format!("Failed to open dropped file: {}", e));
                }
                break;
            }
        }
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }

        let (space, left, right, up, down, f_key) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::Space),
                input.key_pressed(egui::Key::ArrowLeft),
                input.key_pressed(egui::Key::ArrowRight),
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::F),
            )
        });

        if space {
            self.toggle_play();
        }

        if left {
            let mut new_pos = self.current_position - KEYBOARD_SEEK_STEP;
            if new_pos.is_sign_negative() {
                new_pos = 0.0;
            }
            self.seek(new_pos);
        }

        if right {
            let new_pos = (self.current_position + KEYBOARD_SEEK_STEP).min(self.duration.max(0.0));
            self.seek(new_pos);
        }

        if up {
            let new_vol = (self.volume + KEYBOARD_VOLUME_STEP).min(100.0);
            self.set_volume(new_vol);
            self.volume = new_vol;
        }

        if down {
            let new_vol = (self.volume - KEYBOARD_VOLUME_STEP).max(0.0);
            self.set_volume(new_vol);
            self.volume = new_vol;
        }

        if f_key {
            self.is_fullscreen = !self.is_fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.is_fullscreen));
        }
    }

    fn render_room_dialog(&mut self, ctx: &egui::Context) {
        if !self.room_dialog_open {
            return;
        }

        let mut dialog_open = self.room_dialog_open;
        let mut create_room_requested = false;
        let mut join_room_requested = false;
        let mut leave_room_requested = false;

        egui::Window::new("Room Controls")
            .open(&mut dialog_open)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("Display name (shared with the room):");
                ui.text_edit_singleline(&mut self.display_name_input);
                ui.separator();
                if let Some(code) = self.current_room_id.clone() {
                    ui.label(format!("Current room: {}", code));
                    ui.horizontal(|ui| {
                        if ui.button("Copy code").clicked() {
                            ui.output_mut(|o| o.copied_text = code.clone());
                        }
                        if ui.button("Leave room").clicked() {
                            leave_room_requested = true;
                        }
                    });
                    if self.is_host {
                        ui.horizontal(|ui| {
                            if ui.button("Copy invite link").clicked() {
                                let file_name = self
                                    .video_file
                                    .as_ref()
                                    .and_then(|path| path.file_name())
                                    .and_then(|name| name.to_str());
                                let link = invite::build_invite_url(
                                    &code,
                                    self.active_room_passcode.as_deref(),
                                    file_name,
                                );
                                ui.output_mut(|o| o.copied_text = link);
                                self.status_message = "Invite link copied".to_string();
                            }
                            if let Some(passcode) = &self.active_room_passcode {
                                ui.monospace(format!("Passcode: {}", passcode));
                            } else if self.room_has_passcode {
                                ui.label("Passcode protected");
                            } else {
                                ui.label("No passcode set");
                            }
                        });
                        if let Some(limit) = self.room_capacity_limit {
                            ui.label(format!("Capacity: {} seats", limit));
                        }
                    } else if self.room_has_passcode {
                        ui.colored_label(
                            egui::Color32::LIGHT_YELLOW,
                            "Passcode required to rejoin",
                        );
                    }
                    ui.separator();
                    ui.checkbox(&mut self.sync_enabled, "Enable sync");
                    self.draw_participant_indicator(ui);
                } else {
                    ui.label("Create a room to get a sharable 6-digit code.");
                    let can_create = self.video_hash.is_some() && self.sync_connected;
                    if ui
                        .add_enabled(can_create, egui::Button::new("Create Room"))
                        .clicked()
                    {
                        create_room_requested = true;
                    }
                    if !self.sync_connected {
                        ui.colored_label(
                            egui::Color32::LIGHT_YELLOW,
                            "Waiting for sync server (cold starts on Render can take up to a minute)...",
                        );
                    }
                    ui.label("Optional passcode:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.create_passcode_input)
                            .password(true)
                            .hint_text("Leave blank for none"),
                    );
                    ui.add(
                        egui::Slider::new(
                            &mut self.room_capacity_input,
                            ROOM_CAPACITY_MIN..=ROOM_CAPACITY_MAX,
                        )
                        .text("Seats"),
                    );

                    ui.separator();
                    ui.label("Join an existing room:");
                    let response = ui.text_edit_singleline(&mut self.room_id_input);
                    if response.changed() {
                        self.sanitize_room_code_input();
                    }
                    ui.label("Passcode (if needed):");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.join_passcode_input)
                            .password(true)
                            .hint_text("Provided by the host"),
                    );
                    ui.horizontal(|ui| {
                        let can_join = self.video_hash.is_some() && self.sync_connected;
                        if ui.add_enabled(can_join, egui::Button::new("Join")).clicked() {
                            join_room_requested = true;
                        }
                        ui.label("Format: 123-456");
                    });

                    if let Some(session) = self.saved_session.as_ref() {
                        ui.add_space(8.0);
                        ui.separator();
                        let preview_len = session.file_hash.len().min(8);
                        let hash_preview = &session.file_hash[..preview_len];
                        ui.colored_label(
                            egui::Color32::LIGHT_GREEN,
                            format!(
                                "Last room {} (hash {}...)",
                                session.room_id,
                                hash_preview
                            ),
                        );
                        let resume_enabled = self.sync_connected && !self.resume_in_progress;
                        if ui
                            .add_enabled(resume_enabled, egui::Button::new("Resume last session"))
                            .clicked()
                        {
                            self.attempt_resume(false);
                        }
                    }
                }

                ui.separator();
                ui.heading("Current Video");
                if let Some(path) = &self.video_file {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        ui.label(format!("File: {}", name));
                    }
                    if let Some(hash) = &self.video_hash {
                        ui.label(format!("Hash: {}...", &hash[..16]));
                    }
                } else {
                    ui.label("No video loaded");
                }
            });

        if leave_room_requested {
            self.leave_room();
            dialog_open = false;
        }
        if create_room_requested {
            self.create_room();
        }
        if join_room_requested {
            self.join_room();
        }

        self.room_dialog_open = dialog_open;
    }

    fn render_invite_modal(&mut self, ctx: &egui::Context) {
        if !self.invite_modal_open {
            return;
        }

        let Some(invite) = self.pending_invite.as_ref() else {
            self.invite_modal_open = false;
            return;
        };

        let mut modal_open = self.invite_modal_open;
        let mut open_file_requested = false;
        let mut join_requested = false;
        let mut dismiss_requested = false;

        egui::Window::new("Room Invite")
            .open(&mut modal_open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.heading(format!("Join room {}", invite.room_id));
                if let Some(file) = &invite.file_name {
                    ui.label(format!("Expected file: {}", file));
                } else {
                    ui.label("Host did not specify a file name");
                }

                if let Some(passcode) = &invite.passcode {
                    ui.label(format!("Passcode: {}", passcode));
                } else {
                    ui.label("No passcode included");
                }

                ui.separator();

                if let Some(path) = &self.video_file {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        ui.label(format!("Loaded video: {}", name));
                    }
                } else {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        "Load the shared video before joining",
                    );
                }

                ui.add_space(6.0);

                if ui.button("Open Video...").clicked() {
                    open_file_requested = true;
                }

                let join_enabled = self.video_hash.is_some();
                if ui
                    .add_enabled(join_enabled, egui::Button::new("Join Room"))
                    .clicked()
                {
                    join_requested = true;
                }

                if !join_enabled {
                    ui.small("Load the matching video to enable joining");
                }

                if ui.button("Dismiss").clicked() {
                    dismiss_requested = true;
                }
            });

        if dismiss_requested || !modal_open {
            self.invite_modal_open = false;
            self.pending_invite = None;
        }

        if open_file_requested {
            self.select_video_file();
        }

        if join_requested {
            self.join_room();
        }
    }

    fn render_network_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_network_overlay {
            return;
        }
        let mut overlay_open = self.show_network_overlay;
        egui::Window::new("Network Stats")
            .id(egui::Id::new("hang-network-stats"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
            .open(&mut overlay_open)
            .show(ctx, |ui| {
                let stats: SyncStatsSnapshot = self.sync.stats_snapshot();
                let endpoint = stats
                    .endpoint_label
                    .unwrap_or_else(|| "Not connected".to_string());
                ui.label(format!("Endpoint: {}", endpoint));
                ui.separator();
                ui.label(format!(
                    "Traffic: ↓ {} · ↑ {}",
                    Self::format_bytes_short(stats.bytes_in),
                    Self::format_bytes_short(stats.bytes_out)
                ));
                ui.label(format!(
                    "Messages: ↓ {} · ↑ {}",
                    stats.messages_in, stats.messages_out
                ));
                let rtt = stats
                    .last_rtt_ms
                    .map(|ms| format!("{ms:.0} ms"))
                    .unwrap_or_else(|| "—".to_string());
                ui.label(format!("Latency: {}", rtt));
                let last_message = stats
                    .last_message_age
                    .map(|secs| format!("{secs:.1} s ago"))
                    .unwrap_or_else(|| "—".to_string());
                ui.label(format!("Last sync: {}", last_message));
                ui.label(format!(
                    "Connected: {}",
                    Self::describe_duration(stats.connected_duration)
                ));
                ui.label(format!("Reconnect attempts: {}", stats.reconnect_attempts));
                if let Some(disconnect) = stats.last_disconnect_secs {
                    ui.label(format!("Last drop: {:.1} s ago", disconnect));
                }
            });
        self.show_network_overlay = overlay_open;
    }

    fn sanitize_room_code_input(&mut self) {
        let digits: String = self
            .room_id_input
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        if digits.len() <= 3 {
            self.room_id_input = digits;
        } else {
            let (left, right) = digits.split_at(3);
            let right = &right[..right.len().min(3)];
            self.room_id_input = format!("{left}-{right}");
        }
    }

    fn is_valid_room_code(code: &str) -> bool {
        let trimmed = code.trim();
        if trimmed.len() != 7 || trimmed.as_bytes()[3] != b'-' {
            return false;
        }
        trimmed
            .chars()
            .enumerate()
            .all(|(idx, ch)| idx == 3 || ch.is_ascii_digit())
    }

    fn draw_participant_indicator(&self, ui: &mut egui::Ui) {
        if !self.in_room {
            return;
        }
        let capacity_text = self
            .room_capacity_limit
            .map(|limit| format!("{} / {}", self.participant_count.max(1), limit))
            .unwrap_or_else(|| format!("{} online", self.participant_count.max(1)));
        ui.vertical(|ui| {
            ui.label(format!("Participants ({capacity_text})"));
            for member in &self.member_roster {
                let label = if member.is_host {
                    format!("★ {}", member.display_name)
                } else {
                    format!("• {}", member.display_name)
                };
                ui.label(label);
            }
            if self.member_roster.is_empty() {
                ui.label("Waiting for roster update...");
            }
        });
    }

    fn render_track_selectors(&mut self, ui: &mut egui::Ui) {
        if self.audio_tracks.is_empty() && self.subtitle_tracks.is_empty() {
            return;
        }
        ui.horizontal(|ui| {
            if !self.audio_tracks.is_empty() {
                let selected = self
                    .audio_tracks
                    .iter()
                    .find(|track| track.id == self.selected_audio)
                    .map(|track| Self::describe_track(&track.title, &track.lang))
                    .unwrap_or_else(|| "Default".to_string());
                egui::ComboBox::from_label("Audio")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for track in &self.audio_tracks {
                            let label = Self::describe_track(&track.title, &track.lang);
                            if ui
                                .selectable_value(&mut self.selected_audio, track.id, label)
                                .clicked()
                            {
                                if let Err(e) = self.player.set_audio_track(track.id) {
                                    self.error_message =
                                        Some(format!("Failed to switch audio track: {}", e));
                                }
                            }
                        }
                    });
            }

            if !self.subtitle_tracks.is_empty() {
                let selected = if self.selected_subtitle == -1 {
                    "No subtitles".to_string()
                } else {
                    self.subtitle_tracks
                        .iter()
                        .find(|track| track.id == self.selected_subtitle)
                        .map(|track| Self::describe_track(&track.title, &track.lang))
                        .unwrap_or_else(|| "Custom".to_string())
                };
                egui::ComboBox::from_label("Subtitles")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut self.selected_subtitle, -1, "None")
                            .clicked()
                        {
                            if let Err(e) = self.player.set_subtitle_track(-1) {
                                self.error_message =
                                    Some(format!("Failed to disable subtitles: {}", e));
                            }
                        }
                        for track in &self.subtitle_tracks {
                            let label = Self::describe_track(&track.title, &track.lang);
                            if ui
                                .selectable_value(&mut self.selected_subtitle, track.id, label)
                                .clicked()
                            {
                                if let Err(e) = self.player.set_subtitle_track(track.id) {
                                    self.error_message =
                                        Some(format!("Failed to switch subtitle track: {}", e));
                                }
                            }
                        }
                    });
            }
        });
    }

    fn describe_track(title: &str, lang: &str) -> String {
        if lang.is_empty() {
            title.to_string()
        } else {
            format!("{} ({})", title, lang)
        }
    }

    fn update_control_visibility(&mut self, ctx: &egui::Context) {
        if !self.is_fullscreen {
            self.controls_visible = true;
            return;
        }

        let (pointer_pos, screen_rect) = ctx.input(|i| (i.pointer.hover_pos(), i.screen_rect));
        let hover = pointer_pos.map(|pos| {
            let top_zone = screen_rect.top() + 80.0;
            let bottom_zone = screen_rect.bottom() - 120.0;
            pos.y <= top_zone || pos.y >= bottom_zone
        });

        self.controls_visible = hover.unwrap_or(false);
    }
    
    fn draw_loading_spinner(&self, ui: &mut egui::Ui) {
        let time = ui.input(|i| i.time);
        let angle = time * 2.0; // Rotation speed
        
        ui.vertical_centered(|ui| {
            // Draw spinning circle
            let size = 40.0;
            let (response, painter) = ui.allocate_painter(egui::vec2(size, size), egui::Sense::hover());
            let center = response.rect.center();
            let radius = size / 2.0 - 4.0;
            
            // Draw arc segments
            let segments = 8;
            for i in 0..segments {
                let start_angle = angle + (i as f64 * std::f64::consts::TAU / segments as f64);
                let alpha = ((i as f32 / segments as f32) * 200.0) as u8 + 55;
                let color = egui::Color32::from_rgba_unmultiplied(255, 255, 255, alpha);
                
                let p1 = center + egui::vec2(
                    (start_angle.cos() * radius as f64) as f32,
                    (start_angle.sin() * radius as f64) as f32,
                );
                let end_angle = start_angle + std::f64::consts::TAU / segments as f64 * 0.7;
                let p2 = center + egui::vec2(
                    (end_angle.cos() * radius as f64) as f32,
                    (end_angle.sin() * radius as f64) as f32,
                );
                
                painter.line_segment([p1, p2], egui::Stroke::new(3.0, color));
            }
            
            ui.add_space(12.0);
            ui.label(egui::RichText::new("Loading...").color(egui::Color32::WHITE));
        });
    }
    
    fn draw_buffering_overlay(&self, ui: &mut egui::Ui, _available: egui::Vec2) {
        // Get the screen rect to center the overlay properly
        let screen_rect = ui.ctx().screen_rect();
        
        // Semi-transparent overlay in center of screen
        let overlay_size = egui::vec2(120.0, 80.0);
        let overlay_rect = egui::Rect::from_center_size(screen_rect.center(), overlay_size);
        
        // Draw background
        ui.painter().rect_filled(
            overlay_rect,
            8.0,
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180),
        );
        
        // Draw spinner in overlay
        let time = ui.input(|i| i.time);
        let angle = time * 3.0;
        let center = overlay_rect.center();
        let radius = 15.0;
        
        let segments = 8;
        for i in 0..segments {
            let start_angle = angle + (i as f64 * std::f64::consts::TAU / segments as f64);
            let alpha = ((i as f32 / segments as f32) * 200.0) as u8 + 55;
            let color = egui::Color32::from_rgba_unmultiplied(255, 255, 255, alpha);
            
            let p1 = center + egui::vec2(
                (start_angle.cos() * radius as f64) as f32,
                (start_angle.sin() * radius as f64) as f32,
            );
            let end_angle = start_angle + std::f64::consts::TAU / segments as f64 * 0.7;
            let p2 = center + egui::vec2(
                (end_angle.cos() * radius as f64) as f32,
                (end_angle.sin() * radius as f64) as f32,
            );
            
            ui.painter().line_segment([p1, p2], egui::Stroke::new(2.5, color));
        }
        
        // Buffering text
        ui.painter().text(
            center + egui::vec2(0.0, 28.0),
            egui::Align2::CENTER_CENTER,
            "Buffering...",
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE,
        );
    }
}

impl eframe::App for HangApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_playback_state();
        self.update_video_texture(ctx);
        self.handle_file_drop(ctx);
        self.poll_invite_channel();
        self.poll_youtube_loader();
        self.handle_keyboard_shortcuts(ctx);
        
        // Request repaint while YouTube is loading
        if self.youtube_loader.is_some() {
            ctx.request_repaint();
        }
        
        if self.is_fullscreen && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.is_fullscreen = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        self.update_control_visibility(ctx);
        self.maybe_auto_resume();
        let show_chrome = !self.is_fullscreen || self.controls_visible;

        // Top menu bar
        if show_chrome {
            egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
                egui::menu::bar(ui, |ui| {
                    ui.label(
                        egui::RichText::new("Hang")
                            .font(egui::FontId::proportional(20.0))
                            .color(egui::Color32::WHITE),
                    );
                    ui.separator();

                    if ui.button("Open Video").clicked() {
                        self.select_video_file();
                    }
                    
                    if ui.button("Open URL").clicked() {
                        self.show_url_dialog = true;
                    }

                    if ui.button("⚙ Settings").clicked() {
                        self.show_settings = !self.show_settings;
                    }

                    if ui.button("Room Controls").clicked() {
                        self.room_dialog_open = true;
                    }

                    if ui.button("About").clicked() {
                        self.show_about = true;
                    }

                    if ui.button("Network Stats").clicked() {
                        self.show_network_overlay = !self.show_network_overlay;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let connection_label = if self.sync_connected {
                            "Connected"
                        } else {
                            "Connecting..."
                        };
                        let label = ui.label(egui::RichText::new(connection_label).color(
                            if self.sync_connected {
                                egui::Color32::from_rgb(120, 255, 120)
                            } else {
                                egui::Color32::YELLOW
                            },
                        ));
                        label.on_hover_text(&self.status_message);

                        if !self.sync_connected {
                            if ui.button("Retry Connection").clicked() {
                                self.request_manual_reconnect();
                            }
                        } else if !self.in_room {
                            if let Some(session) = self.saved_session.as_ref() {
                                let label = format!("Resume {}", session.room_id);
                                if ui
                                    .add_enabled(!self.resume_in_progress, egui::Button::new(label))
                                    .clicked()
                                {
                                    self.attempt_resume(false);
                                }
                            }
                        }
                    });
                });
            });
        }

        self.render_room_dialog(ctx);
        self.render_invite_modal(ctx);
        self.render_network_overlay(ctx);

        // Bottom control panel
        if show_chrome {
            egui::TopBottomPanel::bottom("controls").show(ctx, |ui| {
                ui.add_space(2.0);

                // Custom full-width timeline
                let mut position = self.current_position;
                let duration = self.duration.max(1.0);
                let is_buffering = self.is_buffering;
                let is_youtube = self.current_youtube_url.is_some();
                
                // Timeline area - full width with more height for better interaction
                let timeline_height = 12.0;
                let available_width = ui.available_width();
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(available_width, timeline_height + 16.0),
                    egui::Sense::click_and_drag(),
                );
                
                if ui.is_rect_visible(rect) {
                    let painter = ui.painter();
                    
                    // Timeline track area (centered vertically)
                    let track_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.min.x, rect.center().y - timeline_height / 2.0),
                        egui::vec2(rect.width(), timeline_height),
                    );
                    
                    // Background track
                    painter.rect_filled(
                        track_rect,
                        4.0,
                        egui::Color32::from_rgb(60, 60, 60),
                    );
                    
                    // Buffering indicator for streaming content
                    if is_youtube || is_buffering {
                        // Show a pulsing buffered area slightly ahead of playback
                        let buffered_ratio = (position / duration + 0.1).min(1.0);
                        let buffered_width = track_rect.width() * buffered_ratio as f32;
                        let buffered_rect = egui::Rect::from_min_size(
                            track_rect.min,
                            egui::vec2(buffered_width, track_rect.height()),
                        );
                        painter.rect_filled(
                            buffered_rect,
                            4.0,
                            egui::Color32::from_rgb(80, 80, 80),
                        );
                    }
                    
                    // Progress bar (played portion)
                    let progress_ratio = if duration > 0.0 { position / duration } else { 0.0 };
                    let progress_width = track_rect.width() * progress_ratio as f32;
                    if progress_width > 0.0 {
                        let progress_rect = egui::Rect::from_min_size(
                            track_rect.min,
                            egui::vec2(progress_width, track_rect.height()),
                        );
                        
                        // Gradient-like effect with main color
                        let progress_color = if is_buffering {
                            egui::Color32::from_rgb(255, 180, 100) // Orange when buffering
                        } else {
                            egui::Color32::from_rgb(100, 180, 255) // Blue normally
                        };
                        painter.rect_filled(progress_rect, 4.0, progress_color);
                    }
                    
                    // Playhead (draggable handle)
                    let handle_x = track_rect.min.x + progress_width;
                    let handle_radius = if response.hovered() || response.dragged() { 8.0 } else { 6.0 };
                    let handle_color = if response.dragged() {
                        egui::Color32::WHITE
                    } else if response.hovered() {
                        egui::Color32::from_rgb(220, 220, 255)
                    } else {
                        egui::Color32::from_rgb(180, 180, 220)
                    };
                    painter.circle_filled(
                        egui::pos2(handle_x, track_rect.center().y),
                        handle_radius,
                        handle_color,
                    );
                    
                    // Buffering animation dots
                    if is_buffering {
                        let time = ui.ctx().input(|i| i.time);
                        let dot_count = 3;
                        for i in 0..dot_count {
                            let phase = (time * 2.0 + i as f64 * 0.3) % 1.0;
                            let alpha = ((phase * std::f64::consts::PI).sin() * 200.0) as u8;
                            let dot_x = handle_x + 15.0 + i as f32 * 8.0;
                            if dot_x < track_rect.max.x - 5.0 {
                                painter.circle_filled(
                                    egui::pos2(dot_x, track_rect.center().y),
                                    3.0,
                                    egui::Color32::from_rgba_unmultiplied(255, 180, 100, alpha),
                                );
                            }
                        }
                        ui.ctx().request_repaint();
                    }
                }
                
                // Handle timeline interaction
                if response.dragged() || response.clicked() {
                    if let Some(pointer_pos) = response.interact_pointer_pos() {
                        let track_start = rect.min.x;
                        let track_width = rect.width();
                        let relative_x = (pointer_pos.x - track_start).max(0.0).min(track_width);
                        position = (relative_x / track_width) as f64 * duration;
                    }
                }
                
                if response.drag_stopped() || response.clicked() {
                    self.seek(position);
                }
                
                // Time display below timeline
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{} / {}",
                        format_time(self.current_position),
                        format_time(self.duration)
                    ));
                    
                    if is_buffering {
                        ui.label(
                            egui::RichText::new("⏳ Buffering...")
                                .color(egui::Color32::from_rgb(255, 180, 100))
                                .small()
                        );
                    }
                });
                
                ui.add_space(2.0);

                ui.horizontal(|ui| {
                    // Play/Pause
                    let play_btn = if self.is_playing { "⏸" } else { "▶" };
                    if ui.button(play_btn).clicked() {
                        self.toggle_play();
                    }

                    // Stop
                    if ui.button("⏹").clicked() {
                        let _ = self.player.stop();
                    }

                    // Frame step
                    if ui.button("⏮").clicked() {
                        let _ = self.player.frame_step_backward();
                    }
                    if ui.button("⏭").clicked() {
                        let _ = self.player.frame_step_forward();
                    }

                    ui.separator();

                    // Speed control
                    ui.label("Speed:");
                    let mut speed = self.speed;
                    if ui
                        .add(egui::Slider::new(&mut speed, 0.25..=2.0).suffix("x"))
                        .changed()
                    {
                        self.set_speed(speed);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Volume
                        let mut vol = self.volume;
                        if ui
                            .add(egui::Slider::new(&mut vol, 0.0..=100.0).text("🔊"))
                            .changed()
                        {
                            self.set_volume(vol);
                        }

                        // Fullscreen
                        let fs_label = if self.is_fullscreen { "🗗" } else { "⛶" };
                        if ui.button(fs_label).clicked() {
                            self.is_fullscreen = !self.is_fullscreen;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                                self.is_fullscreen,
                            ));
                        }
                    });
                });

                ui.add_space(4.0);
                self.render_track_selectors(ui);

                ui.add_space(5.0);

                if self.in_room {
                    ui.separator();
                    self.draw_participant_indicator(ui);
                }

                ui.add_space(4.0);
                ui.small("Keys: Space toggles playback · ←/→ seek 5s · ↑/↓ volume · F fullscreen");
            });
        }

        // Settings window
        if self.show_settings {
            let mut settings_open = self.show_settings;
            let mut quality_changed: Option<youtube::VideoQuality> = None;
            
            egui::Window::new("Settings")
                .open(&mut settings_open)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    // YouTube Quality section (only show when playing YouTube)
                    if self.current_youtube_url.is_some() {
                        ui.heading("🎬 Video Quality");
                        ui.add_space(4.0);
                        
                        ui.horizontal(|ui| {
                            ui.label("Quality:");
                            egui::ComboBox::from_id_salt("settings_youtube_quality")
                                .selected_text(self.youtube_quality.as_str())
                                .show_ui(ui, |ui| {
                                    for quality in youtube::VideoQuality::all() {
                                        if ui.selectable_value(
                                            &mut self.youtube_quality,
                                            *quality,
                                            quality.as_str(),
                                        ).clicked() {
                                            quality_changed = Some(*quality);
                                        }
                                    }
                                });
                        });
                        
                        if self.youtube_loader.is_some() {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Changing quality...");
                            });
                        }
                        
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(4.0);
                    }
                    
                    if ui.button("Refresh Tracks").clicked() {
                        if let Err(e) = self.refresh_media_tracks() {
                            self.error_message = Some(e);
                        }
                    }
                    ui.separator();
                    ui.heading("Audio Tracks");
                    for track in &self.audio_tracks {
                        let label = if track.lang.is_empty() {
                            track.title.clone()
                        } else {
                            format!("{} ({})", track.title, track.lang)
                        };
                        if ui.radio(self.selected_audio == track.id, label).clicked() {
                            self.selected_audio = track.id;
                            let _ = self.player.set_audio_track(track.id);
                        }
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.heading("Subtitle Tracks");

                    if ui.radio(self.selected_subtitle == -1, "None").clicked() {
                        self.selected_subtitle = -1;
                        let _ = self.player.set_subtitle_track(-1);
                    }

                    for track in &self.subtitle_tracks {
                        let label = if track.lang.is_empty() {
                            track.title.clone()
                        } else {
                            format!("{} ({})", track.title, track.lang)
                        };
                        if ui
                            .radio(self.selected_subtitle == track.id, label)
                            .clicked()
                        {
                            self.selected_subtitle = track.id;
                            let _ = self.player.set_subtitle_track(track.id);
                        }
                    }
                });
            self.show_settings = settings_open;
            
            // Handle quality change - reload video with new quality
            if let Some(_new_quality) = quality_changed {
                if let Some(url) = self.current_youtube_url.clone() {
                    // Save current position to resume after quality change
                    let current_pos = self.current_position;
                    self.load_youtube_video_at_position(&url, current_pos);
                }
            }
        }

        if self.show_about {
            let mut about_open = self.show_about;
            let mut close_requested = false;
            let mut check_update_clicked = false;
            let mut download_update_clicked = false;
            
            egui::Window::new("About Hang")
                .collapsible(false)
                .resizable(false)
                .min_width(320.0)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .open(&mut about_open)
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.heading("Hang");
                        ui.label(format!("Version {}", VERSION));
                        ui.add_space(4.0);
                        ui.label("Watch your local videos in sync with friends.");
                        ui.add_space(8.0);
                        
                        // Mode indicator
                        if is_portable_mode() {
                            ui.horizontal(|ui| {
                                ui.label("📦");
                                ui.colored_label(egui::Color32::from_rgb(100, 200, 100), "Portable Mode");
                            });
                        }
                        
                        // Data directory
                        if let Some(data_dir) = get_data_directory() {
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label("Data:");
                                ui.monospace(data_dir.display().to_string());
                            });
                        }
                        
                        ui.separator();
                        
                        // Update section
                        ui.add_space(4.0);
                        if let Some(update) = &self.update_info {
                            if update.is_update_available {
                                ui.colored_label(
                                    egui::Color32::from_rgb(255, 200, 100),
                                    format!("🎉 Update available: v{}", update.latest_version)
                                );
                                ui.add_space(4.0);
                                if ui.button("⬇ Download Update").clicked() {
                                    download_update_clicked = true;
                                }
                            } else {
                                ui.colored_label(
                                    egui::Color32::from_rgb(100, 200, 100),
                                    "✓ You're on the latest version"
                                );
                            }
                        } else if self.update_check_done {
                            ui.label("Could not check for updates");
                        } else {
                            if ui.button("🔄 Check for Updates").clicked() {
                                check_update_clicked = true;
                            }
                        }
                        
                        ui.add_space(8.0);
                        ui.separator();
                        
                        // Connection status
                        let status = if self.sync_connected {
                            "🟢 Connected"
                        } else {
                            "🟡 Connecting..."
                        };
                        ui.label(format!("Sync: {}", status));
                        
                        ui.add_space(8.0);
                        if ui.button("Close").clicked() {
                            close_requested = true;
                        }
                    });
                });
            self.show_about = about_open && !close_requested;
            
            if check_update_clicked {
                self.check_for_updates();
            }
            if download_update_clicked {
                if let Some(update) = &self.update_info {
                    update::open_download_page(&update.download_url);
                }
            }
        }
        
        // URL Input Dialog
        if self.show_url_dialog {
            let mut load_url = false;
            let mut close_dialog = false;
            let is_youtube = youtube::is_youtube_url(&self.url_input);
            
            egui::Window::new("Open URL")
                .collapsible(false)
                .resizable(false)
                .min_width(420.0)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Enter a video URL:");
                    ui.add_space(4.0);
                    
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.url_input)
                            .hint_text("https://youtube.com/watch?v=... or direct URL")
                            .desired_width(400.0)
                    );
                    
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        load_url = true;
                    }
                    
                    // Quality selector for YouTube URLs
                    if is_youtube {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("Quality:");
                            egui::ComboBox::from_id_salt("youtube_quality")
                                .selected_text(self.youtube_quality.as_str())
                                .show_ui(ui, |ui| {
                                    for quality in youtube::VideoQuality::all() {
                                        ui.selectable_value(
                                            &mut self.youtube_quality,
                                            *quality,
                                            quality.as_str(),
                                        );
                                    }
                                });
                        });
                    }
                    
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("▶ Load").clicked() {
                            load_url = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close_dialog = true;
                        }
                    });
                    
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    
                    // Show supported formats
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("📁 Direct URLs:").small().strong());
                        ui.label(egui::RichText::new("MP4, MKV, AVI, MOV, WebM").small().color(egui::Color32::GRAY));
                    });
                    
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("▶ YouTube:").small().strong());
                        ui.label(egui::RichText::new("✓ Supported (auto-downloads yt-dlp)").small().color(egui::Color32::from_rgb(100, 200, 100)));
                    });
                });
            
            if close_dialog {
                self.show_url_dialog = false;
            }
            
            if load_url && !self.url_input.trim().is_empty() {
                self.load_video_from_url();
            }
        }

        // Error notification
        if let Some(error) = self.error_message.clone() {
            egui::Window::new("Error")
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.colored_label(egui::Color32::RED, &error);
                    if ui.button("OK").clicked() {
                        self.error_message = None;
                    }
                });
        }

        // Central panel (embedded video output)
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                let available = ui.available_size();
                
                if let Some(texture) = &self.video_texture {
                    let draw_size = self.fitted_video_size(available);
                    ui.allocate_ui_with_layout(
                        available,
                        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                        |ui| {
                            ui.image((texture.id(), draw_size));
                        },
                    );
                    
                    // Show buffering overlay on top of video
                    if self.is_buffering {
                        self.draw_buffering_overlay(ui, available);
                    }
                } else if self.video_file.is_some() || self.youtube_loader.is_some() {
                    ui.allocate_ui_with_layout(
                        available,
                        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                        |ui| {
                            self.draw_loading_spinner(ui);
                        },
                    );
                }
            });

        // Request continuous repaint for smooth updates
        ctx.request_repaint();
    }
}
