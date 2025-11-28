use eframe::egui;
use parking_lot::Mutex;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{
    constants::{LOCAL_WS_URL, RENDER_WS_URL},
    player::{VideoFrame, VideoPlayer},
    protocol::{Message, SyncCommand},
    sync::SyncClient,
    utils::{compute_file_hash, format_time},
};

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v",
];

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
    current_room_id: Option<String>,
    is_host: bool,
    participant_count: usize,
    room_dialog_open: bool,

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
            participant_count: 0,
            room_dialog_open: false,
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

        self.audio_tracks = self.player.get_audio_tracks().unwrap_or_default();
        self.subtitle_tracks = self.player.get_subtitle_tracks().unwrap_or_default();

        if let Err(e) = self.player.play() {
            self.error_message = Some(format!("Failed to auto-play: {}", e));
        } else {
            self.is_playing = true;
        }

        Ok(())
    }

    fn first_supported_file(folder: &Path) -> Option<PathBuf> {
        let entries = fs::read_dir(folder).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && Self::is_supported_video(&path) {
                return Some(path);
            }
        }
        None
    }

    fn is_supported_video(path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| VIDEO_EXTENSIONS.iter().any(|allowed| ext.eq_ignore_ascii_case(allowed)))
            .unwrap_or(false)
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
            .add_filter("Video Files", VIDEO_EXTENSIONS)
            .pick_file()
        {
            if let Err(e) = self.load_video_from_path(&path) {
                self.error_message = Some(format!("Failed to load video: {}", e));
            }
        }
    }

    fn select_video_folder(&mut self) {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            match Self::first_supported_file(&folder) {
                Some(file) => {
                    if let Err(e) = self.load_video_from_path(&file) {
                        self.error_message = Some(format!("Failed to load video: {}", e));
                    }
                }
                None => {
                    self.error_message = Some("No supported video files found in that folder".into());
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
            if let Err(e) = self.sync.join_room(code.clone(), hash.clone()) {
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
                self.sync
                    .set_room_joined(room_id.clone(), client_id, true);
                self.in_room = true;
                self.current_room_id = Some(room_id.clone());
                self.is_host = true;
                self.participant_count = 1;
                self.status_message = format!("Room created: {}", room_id);
                self.room_id_input = room_id;
            }
            Message::RoomJoined {
                room_id,
                client_id,
                is_host,
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
            }
            Message::RoomLeft => {
                self.sync.clear_room();
                self.in_room = false;
                self.current_room_id = None;
                self.is_host = false;
                self.participant_count = 0;
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
            Message::RoomMemberUpdate { room_id, members } => {
                if self.current_room_id.as_deref() == Some(room_id.as_str()) {
                    self.participant_count = members;
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
                if let Err(e) = self.load_video_from_path(&path) {
                    self.error_message = Some(format!("Failed to open dropped file: {}", e));
                }
                break;
            }
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
            .show(ctx, |ui| {
                ui.label("Server URL:");
                ui.text_edit_singleline(&mut self.server_url);
                ui.small(format!("Fallback: {}", RENDER_WS_URL));
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
                    ui.separator();
                    ui.checkbox(&mut self.sync_enabled, "Enable sync");
                    ui.label(format!(
                        "Participants detected: {}",
                        self.participant_count.max(1)
                    ));
                } else {
                    ui.label("Create a room to get a sharable 6-digit code.");
                    if ui
                        .add_enabled(
                            self.video_hash.is_some(),
                            egui::Button::new("Create Room"),
                        )
                        .clicked()
                    {
                        create_room_requested = true;
                    }

                    ui.separator();
                    ui.label("Join an existing room:");
                    let response = ui.text_edit_singleline(&mut self.room_id_input);
                    if response.changed() {
                        self.sanitize_room_code_input();
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(self.video_hash.is_some(), egui::Button::new("Join"))
                            .clicked()
                        {
                            join_room_requested = true;
                        }
                        ui.label("Format: 123-456");
                    });
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
        let count = self.participant_count.max(1);
        ui.horizontal(|ui| {
            ui.label("Participants:");
            for idx in 0..count {
                let color = if idx == 0 {
                    egui::Color32::from_rgb(120, 200, 120)
                } else {
                    egui::Color32::from_rgb(58, 198, 86)
                };
                ui.colored_label(color, "●");
            }
        });
    }
}

impl eframe::App for HangApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_playback_state();
        self.update_video_texture(ctx);
        self.handle_file_drop(ctx);

        // Top menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.heading("🎬 Hang Sync Player");
                ui.separator();

                if ui.button("Open Video").clicked() {
                    self.select_video_file();
                }

                if ui.button("Open Folder").clicked() {
                    self.select_video_folder();
                }

                if ui.button("⚙ Settings").clicked() {
                    self.show_settings = !self.show_settings;
                }

                if ui.button("Room Controls").clicked() {
                    self.room_dialog_open = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(&self.status_message);
                });
            });
        });

        self.render_room_dialog(ctx);

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

            if self.in_room {
                ui.separator();
                self.draw_participant_indicator(ui);
            }
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
                        ui.add_space(10.0);
                        if ui.button("Open Video").clicked() {
                            self.select_video_file();
                        }
                        if ui.button("Open Folder").clicked() {
                            self.select_video_folder();
                        }
                        ui.label("…or drag & drop a file anywhere in this window");
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
