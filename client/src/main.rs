mod player;
mod protocol;
mod sync;
mod ui;
mod utils;

use anyhow::Result;
use parking_lot::Mutex;
use std::sync::Arc;

use player::VideoPlayer;
use sync::SyncClient;
use ui::HangApp;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hang_client=debug,info".into()),
        )
        .init();

    // Initialize video player
    let player = Arc::new(VideoPlayer::new(None).map_err(|e| anyhow::anyhow!(e))?);

    // Initialize sync client
    let sync = Arc::new(SyncClient::new());

    // Store app state for message handling
    let app_state = Arc::new(Mutex::new(None::<Arc<Mutex<HangApp>>>));
    let app_state_clone = Arc::clone(&app_state);

    // Connect to sync server
    let sync_clone = Arc::clone(&sync);
    tokio::spawn(async move {
        let result = sync_clone
            .connect("ws://localhost:3005/ws", move |msg| {
                if let Some(app_arc) = app_state_clone.lock().as_ref() {
                    let mut app = app_arc.lock();
                    app.handle_server_message(msg);
                }
            })
            .await;

        if let Err(e) = result {
            tracing::error!("Failed to connect to server: {}", e);
        }
    });

    // Give connection time to establish
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Launch GUI on main thread
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 800.0])
            .with_min_inner_size([1000.0, 600.0])
            .with_title("Hang Sync Player"),
        ..Default::default()
    };

    let player_clone = Arc::clone(&player);
    let sync_clone = Arc::clone(&sync);
    let app_state_clone = Arc::clone(&app_state);

    eframe::run_native(
        "Hang Sync Player",
        options,
        Box::new(move |cc| {
            let app = HangApp::new(cc, player_clone, sync_clone);
            let app_arc = Arc::new(Mutex::new(app));
            *app_state_clone.lock() = Some(Arc::clone(&app_arc));

            Ok(Box::new(AppWrapper { app: app_arc }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {}", e))?;

    Ok(())
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
