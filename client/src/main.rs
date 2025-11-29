mod constants;
mod invite;
mod ipc;
mod player;
mod protocol;
mod sync;
mod ui;
mod utils;

use anyhow::Result;
use parking_lot::Mutex;
use std::sync::Arc;

use constants::{LOCAL_WS_URL, RENDER_WS_URL};
use invite::InviteSignal;
use player::VideoPlayer;
use sync::SyncClient;
use tokio::sync::mpsc;
use ui::HangApp;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hang_client=debug,info".into()),
        )
        .init();

    // Parse invite argument if present
    let invite_arg = extract_invite_argument();

    // Set up invite dispatch channel and IPC listener
    let (invite_tx, invite_rx) = mpsc::unbounded_channel::<InviteSignal>();
    let primary_instance = ipc::start_invite_listener(invite_tx.clone()).await;

    if !primary_instance {
        if let Some(url) = invite_arg {
            let _ = ipc::send_invite_to_primary(InviteSignal { url }).await;
        }
        return Ok(());
    }

    if let Some(url) = invite_arg {
        let _ = invite_tx.send(InviteSignal { url });
    }

    // Initialize video player
    let player = Arc::new(VideoPlayer::new(None).map_err(|e| anyhow::anyhow!(e))?);

    // Initialize sync client
    let sync = Arc::new(SyncClient::new());

    // Store app state for message handling
    let app_state = Arc::new(Mutex::new(None::<Arc<Mutex<HangApp>>>));

    // Connect to sync server, preferring localhost with Render fallback
    let sync_for_connection = Arc::clone(&sync);
    let app_state_for_connection = Arc::clone(&app_state);
    tokio::spawn(async move {
        let endpoints = [
            ("local development", LOCAL_WS_URL),
            ("Render deployment", RENDER_WS_URL),
        ];

        for (label, url) in endpoints {
            let handler_state = Arc::clone(&app_state_for_connection);
            match sync_for_connection
                .connect(url, move |msg| {
                    if let Some(app_arc) = handler_state.lock().as_ref() {
                        let mut app = app_arc.lock();
                        app.handle_server_message(msg);
                    }
                })
                .await
            {
                Ok(_) => {
                    tracing::info!("Connected to {label} sync server at {url}");
                    return;
                }
                Err(e) => {
                    tracing::warn!("Failed to connect to {label} sync server at {url}: {}", e)
                }
            }
        }

        tracing::error!(
            "Unable to reach either the local server ({}) or Render backend ({}).",
            LOCAL_WS_URL,
            RENDER_WS_URL
        );
    });

    // Give connection time to establish
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Launch GUI on main thread
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 800.0])
            .with_min_inner_size([1000.0, 600.0])
            .with_title("Hang")
            .with_icon(hang_icon()),
        ..Default::default()
    };
    options.renderer = eframe::Renderer::Glow;

    let player_clone = Arc::clone(&player);
    let sync_clone = Arc::clone(&sync);
    let app_state_clone = Arc::clone(&app_state);
    let mut invite_rx = Some(invite_rx);

    eframe::run_native(
        "Hang",
        options,
        Box::new(move |cc| {
            let invites = invite_rx.take().expect("invite receiver already consumed");
            let app = HangApp::new(cc, player_clone, sync_clone, invites);
            let app_arc = Arc::new(Mutex::new(app));
            *app_state_clone.lock() = Some(Arc::clone(&app_arc));

            Ok(Box::new(AppWrapper { app: app_arc }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {}", e))?;

    Ok(())
}

fn extract_invite_argument() -> Option<String> {
    let mut args = std::env::args().skip(1);
    let mut invite = None;
    while let Some(arg) = args.next() {
        if arg == "--invite-url" {
            invite = args.next();
        } else if arg.starts_with("hang://")
            || arg.starts_with("http://")
            || arg.starts_with("https://")
        {
            invite = Some(arg);
        }
    }
    invite
}

// Wrapper to make Arc<Mutex<HangApp>> work with eframe::App
struct AppWrapper {
    app: Arc<Mutex<HangApp>>,
}

impl eframe::App for AppWrapper {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.lock().update(ctx, frame);
    }
}

fn hang_icon() -> egui::IconData {
    const W: usize = 32;
    const H: usize = 32;
    let top = [255.0, 138.0, 0.0];
    let bottom = [255.0, 108.0, 0.0];
    let mut rgba = Vec::with_capacity(W * H * 4);

    for y in 0..H {
        let t = y as f32 / (H.saturating_sub(1)) as f32;
        let base = [
            (top[0] + (bottom[0] - top[0]) * t) as u8,
            (top[1] + (bottom[1] - top[1]) * t) as u8,
            (top[2] + (bottom[2] - top[2]) * t) as u8,
            255,
        ];

        for x in 0..W {
            let mut color = base;
            let in_left_bar = (7..=11).contains(&x);
            let in_right_bar = (20..=24).contains(&x);
            let in_cross_bar = (11..=21).contains(&x) && (13..=19).contains(&y);
            if in_left_bar || in_right_bar || in_cross_bar {
                color = [255, 255, 255, 255];
            }
            rgba.extend_from_slice(&color);
        }
    }

    egui::IconData {
        rgba,
        width: W as u32,
        height: H as u32,
    }
}
