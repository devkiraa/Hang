use eframe::egui;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    constants::{LOCAL_WS_URL, RENDER_WS_URL},
    player::{VideoFrame, VideoPlayer},
    protocol::{Message, SyncCommand},
    sync::SyncClient,
    utils::{compute_file_hash, format_time},
};

pub struct HangApp {
    // Video player
    player: Arc<VideoPlayer>,

    // Sync client
    sync: Arc<SyncClient>,

    // UI state
    video_file: Option<PathBuf>,
    video_hash: Option<String>,
    server_url: String,
    room_id_input: String,
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
    current_room_id: Option<Uuid>,
    is_host: bool,

    // Settings panel
    show_settings: bool,
    audio_tracks: Vec<crate::player::AudioTrack>,
    subtitle_tracks: Vec<crate::player::SubtitleTrack>,
    selected_audio: i64,
    selected_subtitle: i64,

    // Sync control
    sync_enabled: bool,
    last_sync_time: Arc<Mutex<std::time::Instant>>,

    // Video rendering
    video_texture: Option<egui::TextureHandle>,
    last_frame_size: Option<(u32, u32)>,
}

impl HangApp {
    pub fn new(
        _cc: &eframe::CreationContext,
        player: Arc<VideoPlayer>,
        sync: Arc<SyncClient>,
    ) -> Self {
        Self {
            player,
            sync,
            video_file: None,
            video_hash: None,
            server_url: LOCAL_WS_URL.to_string(),
            room_id_input: String::new(),
            status_message: "Select a video file to begin".to_string(),
            error_message: None,
            is_playing: false,
            current_position: 0.0,
            duration: 0.0,
            volume: 100.0,
            speed: 1.0,
            in_room: false,
            current_room_id: None,
            is_host: false,
            show_settings: false,
            audio_tracks: Vec::new(),
            subtitle_tracks: Vec::new(),
            selected_audio: -1,
            selected_subtitle: -1,
            sync_enabled: true,
            last_sync_time: Arc::new(Mutex::new(std::time::Instant::now())),
            video_texture: None,
            last_frame_size: None,
        }
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

    fn select_video_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                "Video Files",
                &["mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v"],
            )
            .pick_file()
        {
            match self.player.load_file(&path) {
                Ok(_) => {
                    // Compute file hash
                    match compute_file_hash(&path) {
                        Ok(hash) => {
                            self.video_file = Some(path.clone());
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

                            // Load tracks
                            self.audio_tracks = self.player.get_audio_tracks().unwrap_or_default();
                            self.subtitle_tracks =
                                self.player.get_subtitle_tracks().unwrap_or_default();

                            if let Err(e) = self.player.play() {
                                self.error_message = Some(format!("Failed to auto-play: {}", e));
                            } else {
                                self.is_playing = true;
                            }
                        }
                        Err(e) => {
                            self.error_message = Some(format!("Failed to hash file: {}", e));
                        }
                    }
                }
                Err(e) => {
                    self.error_message = Some(format!("Failed to load video: {}", e));
                }
            }
        }
    }

    fn create_room(&mut self) {
        if let Some(hash) = &self.video_hash {
            if let Err(e) = self.sync.create_room(hash.clone()) {
                self.error_message = Some(format!("Failed to create room: {}", e));
            } else {
                self.status_message = "Creating room...".to_string();
            }
        }
    }

    fn join_room(&mut self) {
        if let (Some(hash), Ok(room_id)) = (&self.video_hash, Uuid::parse_str(&self.room_id_input))
        {
            if let Err(e) = self.sync.join_room(room_id, hash.clone()) {
                self.error_message = Some(format!("Failed to join room: {}", e));
            } else {
                self.status_message = "Joining room...".to_string();
            }
        } else {
            self.error_message = Some("Invalid room ID or no video loaded".to_string());
        }
    }

    fn leave_room(&mut self) {
        if let Err(e) = self.sync.leave_room() {
            self.error_message = Some(format!("Failed to leave room: {}", e));
        }
        self.in_room = false;
        self.current_room_id = None;
        self.is_host = false;
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
        if let Err(e) = self.player.seek(position) {
            self.error_message = Some(format!("Seek error: {}", e));
        } else if self.sync_enabled && self.in_room {
            let _ = self.sync.send_sync_command(SyncCommand::Seek {
                timestamp: position,
            });
        }
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
            Message::RoomCreated { room_id, client_id } => {
                self.sync.set_room_joined(room_id, client_id, true);
                self.in_room = true;
                self.current_room_id = Some(room_id);
                self.is_host = true;
                self.status_message = format!("Room created: {}", room_id);
                self.room_id_input = room_id.to_string();
            }
            Message::RoomJoined {
                room_id,
                client_id,
                is_host,
            } => {
                self.sync.set_room_joined(room_id, client_id, is_host);
                self.in_room = true;
                self.current_room_id = Some(room_id);
                self.is_host = is_host;
                self.status_message = format!(
                    "Joined room: {} ({})",
                    room_id,
                    if is_host { "Host" } else { "Guest" }
                );
            }
            Message::RoomLeft => {
                self.sync.clear_room();
                self.in_room = false;
                self.current_room_id = None;
                self.is_host = false;
                self.status_message = "Left room".to_string();
            }
            Message::RoomNotFound => {
                self.error_message = Some("Room not found".to_string());
            }
            Message::FileHashMismatch { expected } => {
                self.error_message =
                    Some(format!("File mismatch! Expected hash: {}", &expected[..16]));
            }
            Message::SyncBroadcast { command, .. } => {
                if self.sync_enabled {
                    self.handle_sync_command(command);
                }
            }
            Message::Error { message } => {
                self.error_message = Some(message);
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
}

impl eframe::App for HangApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_playback_state();
        self.update_video_texture(ctx);

        // Top menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.heading("🎬 Hang Sync Player");
                ui.separator();

                if ui.button("Open Video").clicked() {
                    self.select_video_file();
                }

                if ui.button("⚙ Settings").clicked() {
                    self.show_settings = !self.show_settings;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(&self.status_message);
                });
            });
        });

        // Side panel for room controls
        egui::SidePanel::left("room_panel")
            .min_width(250.0)
            .show(ctx, |ui| {
                ui.heading("Room Controls");
                ui.separator();

                ui.label("Server URL:");
                ui.text_edit_singleline(&mut self.server_url);
                ui.small(format!("Fallback: {}", RENDER_WS_URL));
                ui.add_space(10.0);

                if !self.in_room {
                    ui.label("Create or Join Room:");
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.video_file.is_some(),
                                egui::Button::new("Create Room"),
                            )
                            .clicked()
                        {
                            self.create_room();
                        }
                    });

                    ui.add_space(5.0);
                    ui.label("Room ID:");
                    ui.text_edit_singleline(&mut self.room_id_input);
                    if ui
                        .add_enabled(
                            self.video_file.is_some() && !self.room_id_input.is_empty(),
                            egui::Button::new("Join Room"),
                        )
                        .clicked()
                    {
                        self.join_room();
                    }
                } else {
                    ui.label(format!(
                        "Room: {}",
                        self.current_room_id
                            .map(|id| id.to_string())
                            .unwrap_or_default()
                    ));
                    ui.label(format!(
                        "Role: {}",
                        if self.is_host { "Host" } else { "Guest" }
                    ));

                    ui.add_space(10.0);
                    ui.checkbox(&mut self.sync_enabled, "Enable Sync");

                    ui.add_space(10.0);
                    if ui.button("Leave Room").clicked() {
                        self.leave_room();
                    }
                }

                ui.add_space(20.0);
                ui.separator();
                ui.heading("Current Video");

                if let Some(path) = &self.video_file {
                    ui.label(format!(
                        "File: {}",
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown")
                    ));
                    if let Some(hash) = &self.video_hash {
                        ui.label(format!("Hash: {}...", &hash[..16]));
                    }
                } else {
                    ui.label("No video loaded");
                }
            });

        // Bottom control panel
        egui::TopBottomPanel::bottom("controls").show(ctx, |ui| {
            ui.add_space(5.0);

            // Timeline slider
            let mut position = self.current_position;
            let slider = ui.add(
                egui::Slider::new(&mut position, 0.0..=self.duration.max(1.0))
                    .show_value(false)
                    .text("Timeline"),
            );

            if slider.drag_stopped() || slider.clicked() {
                self.seek(position);
            }

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

                // Time display
                ui.label(format!(
                    "{} / {}",
                    format_time(self.current_position),
                    format_time(self.duration)
                ));

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
                    if ui.button("⛶").clicked() {
                        let _ = self.player.toggle_fullscreen();
                    }
                });
            });

            ui.add_space(5.0);
        });

        // Settings window
        if self.show_settings {
            egui::Window::new("Settings")
                .open(&mut self.show_settings)
                .show(ctx, |ui| {
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
        }

        // Error notification
        if let Some(error) = self.error_message.clone() {
            egui::Window::new("Error")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.colored_label(egui::Color32::RED, &error);
                    if ui.button("OK").clicked() {
                        self.error_message = None;
                    }
                });
        }

        // Central panel (embedded video output)
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(texture) = &self.video_texture {
                let available = ui.available_size();
                let draw_size = self.fitted_video_size(available);
                ui.allocate_ui_with_layout(
                    available,
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        ui.image((texture.id(), draw_size));
                    },
                );
            } else {
                ui.centered_and_justified(|ui| {
                    if self.video_file.is_none() {
                        ui.heading("Open a video file to begin");
                    } else {
                        ui.heading("Loading video...");
                    }
                });
            }
        });

        // Request continuous repaint for smooth updates
        ctx.request_repaint();
    }
}
