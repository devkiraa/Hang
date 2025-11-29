#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

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
use tokio::{sync::mpsc, time::Duration};
use ui::HangApp;
use url::Url;

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
    let (reconnect_tx, reconnect_rx) = mpsc::unbounded_channel::<()>();
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
    tokio::spawn(run_connection_loop(
        sync_for_connection,
        app_state_for_connection,
        reconnect_rx,
    ));

    // Give connection time to establish
    tokio::time::sleep(Duration::from_millis(100)).await;

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
    let reconnect_tx_for_ui = reconnect_tx.clone();

    eframe::run_native(
        "Hang",
        options,
        Box::new(move |cc| {
            let invites = invite_rx.take().expect("invite receiver already consumed");
            let app = HangApp::new(
                cc,
                Arc::clone(&player_clone),
                Arc::clone(&sync_clone),
                invites,
                reconnect_tx_for_ui.clone(),
            );
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

async fn run_connection_loop(
    sync_client: Arc<SyncClient>,
    app_state: Arc<Mutex<Option<Arc<Mutex<HangApp>>>>>,
    mut reconnect_rx: mpsc::UnboundedReceiver<()>,
) {
    let mut attempt: u32 = 0;
    let endpoints = connection_endpoints();

    'outer: loop {
        for (label, url) in endpoints.iter().copied() {
            attempt += 1;
            update_connection_status(
                &app_state,
                format!("Connecting to {label} sync server (attempt {attempt})..."),
                Some(false),
            );

            if label == "Render deployment" {
                warm_up_backend(&app_state, label, url).await;
            }

            let handler_state = Arc::clone(&app_state);
            match sync_client
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
                    update_connection_status(
                        &app_state,
                        format!("Connected to {label} sync server"),
                        Some(true),
                    );
                    return;
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to connect to {label} sync server at {url}: {}",
                        e
                    );
                    update_connection_status(
                        &app_state,
                        format!(
                            "{label} sync server unavailable ({e}). Retrying with fallback..."
                        ),
                        Some(false),
                    );
                }
            }

            let capped_attempt = attempt.min(6);
            let delay = Duration::from_secs(5 * capped_attempt as u64);
            let sleep = tokio::time::sleep(delay);
            tokio::pin!(sleep);
            tokio::select! {
                _ = sleep.as_mut() => {},
                recv = reconnect_rx.recv() => {
                    if recv.is_none() {
                        tracing::info!("Reconnect channel closed; stopping connection loop");
                        return;
                    }
                    tracing::info!("Manual reconnect requested; restarting connection attempts");
                    attempt = 0;
                    continue 'outer;
                }
            }
        }
    }
}

fn connection_endpoints() -> Vec<(&'static str, &'static str)> {
    let disable_local = std::env::var("HANG_DISABLE_LOCAL").is_ok();
    let prefer_local = std::env::var("HANG_PREFER_LOCAL").is_ok();

    let mut endpoints = Vec::with_capacity(2);
    if prefer_local && !disable_local {
        endpoints.push(("local development", LOCAL_WS_URL));
    }
    endpoints.push(("Render deployment", RENDER_WS_URL));
    if !prefer_local && !disable_local {
        endpoints.push(("local development", LOCAL_WS_URL));
    }
    endpoints
}

async fn warm_up_backend(
    app_state: &Arc<Mutex<Option<Arc<Mutex<HangApp>>>>>,
    label: &str,
    ws_url: &str,
) {
    if let Some(health_url) = health_url_from_ws(ws_url) {
        update_connection_status(
            app_state,
            format!("Warming up {label} backend..."),
            Some(false),
        );

        let client = reqwest::Client::new();
        match client
            .get(&health_url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(response) => {
                tracing::info!(
                    "Warmup request to {label} backend at {} returned {}",
                    health_url,
                    response.status()
                );
                update_connection_status(
                    app_state,
                    format!(
                        "{label} backend responded with {}. Attempting WebSocket...",
                        response.status()
                    ),
                    Some(false),
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Warmup request to {label} backend at {} failed: {}",
                    health_url,
                    e
                );
                update_connection_status(
                    app_state,
                    format!(
                        "Warmup attempt for {label} backend failed ({e}). Retrying WebSocket..."
                    ),
                    Some(false),
                );
            }
        }
    }
}

fn health_url_from_ws(ws_url: &str) -> Option<String> {
    let parsed = Url::parse(ws_url).ok()?;
    let scheme = match parsed.scheme() {
        "ws" => "http",
        "wss" => "https",
        _ => return None,
    };

    let mut http = parsed;
    http.set_scheme(scheme).ok()?;
    http.set_path("/healthz");
    http.set_query(None);
    http.set_fragment(None);
    Some(http.to_string())
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

fn update_connection_status(
    app_state: &Arc<Mutex<Option<Arc<Mutex<HangApp>>>>>,
    message: String,
    connected: Option<bool>,
) {
    if let Some(app_arc) = app_state.lock().as_ref() {
        let mut app = app_arc.lock();
        app.update_sync_status(message, connected);
    }
}
