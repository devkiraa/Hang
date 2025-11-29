use axum::{
    extract::{
        ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, RwLock};
use url::form_urlencoded;
use uuid::Uuid;

mod protocol;
mod state;

use protocol::{Message, SyncCommand};
use state::ServerState;

type ClientSender = mpsc::UnboundedSender<Message>;
type ClientSenders = Arc<RwLock<HashMap<Uuid, ClientSender>>>;

const LOG_TAG: &str = "[Hang Server]";

#[derive(Clone)]
struct AppState {
    server_state: ServerState,
    client_senders: ClientSenders,
}

const INDEX_HTML: &str = include_str!("../static/index.html");
const THANK_YOU_HTML: &str = include_str!("../static/thank-you.html");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hang_server=debug,info".into()),
        )
        .init();

    let port = env::var("PORT")
        .ok()
        .and_then(|val| val.parse().ok())
        .unwrap_or(3005);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let app_state = AppState {
        server_state: ServerState::new(),
        client_senders: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/", get(serve_index))
        .route("/thank-you", get(serve_thank_you))
        .route("/thank-you.html", get(serve_thank_you))
        .route("/healthz", get(health_check))
        .route("/ws", get(ws_endpoint))
        .route("/join", get(join_page))
        .route("/join/:room_id", get(join_page_with_path))
        .with_state(app_state.clone());

    let listener = TcpListener::bind(addr).await?;
    tracing::info!("{LOG_TAG} Listening on {}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_endpoint(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_connection(socket, state))
}

async fn serve_index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn serve_thank_you() -> Html<&'static str> {
    Html(THANK_YOU_HTML)
}

async fn health_check() -> &'static str {
    "ok"
}

#[derive(Debug, Default, Deserialize)]
struct InviteQuery {
    room: Option<String>,
    code: Option<String>,
    file: Option<String>,
}

async fn join_page(Query(query): Query<InviteQuery>) -> Html<String> {
    Html(render_join_page(query.room, query.code, query.file))
}

async fn join_page_with_path(
    Path(room_id): Path<String>,
    Query(mut query): Query<InviteQuery>,
) -> Html<String> {
    if query.room.is_none() {
        query.room = Some(room_id);
    }
    Html(render_join_page(query.room, query.code, query.file))
}

async fn handle_connection(socket: WebSocket, state: AppState) {
    let server_state = state.server_state.clone();
    let client_senders = state.client_senders.clone();
    let client_id = Uuid::new_v4();
    server_state.add_client(client_id);

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    // Register client sender
    client_senders.write().await.insert(client_id, tx.clone());

    // Spawn task to send messages to client
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let json = match serde_json::to_string(&msg) {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!("Failed to serialize message: {}", e);
                    continue;
                }
            };

            if let Err(e) = ws_sender.send(AxumWsMessage::Text(json)).await {
                tracing::error!("Failed to send message: {}", e);
                break;
            }
        }
    });

    // Handle incoming messages
    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(AxumWsMessage::Text(text)) => {
                if let Err(e) =
                    handle_message(&text, client_id, &server_state, &client_senders).await
                {
                    tracing::error!("Error handling message: {}", e);
                    let _ = tx.send(Message::Error {
                        message: e.to_string(),
                    });
                }
            }
            Ok(AxumWsMessage::Close(_)) => {
                tracing::info!("Client {} closing connection", client_id);
                break;
            }
            Err(e) => {
                tracing::error!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    // Cleanup
    client_senders.write().await.remove(&client_id);
    server_state.remove_client(client_id).await;
    send_task.abort();
}

fn render_join_page(room: Option<String>, code: Option<String>, file: Option<String>) -> String {
    let room = room.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let code = code.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let file = file.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let heading = room
        .as_ref()
        .map(|room_id| format!("Join Hang Room {}", html_escape(room_id)))
        .unwrap_or_else(|| "Hang Invite".to_string());

    let passcode_block = code
        .as_ref()
        .map(|value| {
            format!(
                "<div class=\"info\">Passcode: <code>{}</code></div>",
                html_escape(value)
            )
        })
        .unwrap_or_else(|| {
            "<div class=\"info muted\">No passcode included in this invite.</div>".to_string()
        });

    let file_block = file
        .as_ref()
        .map(|value| {
            format!(
                "<div class=\"info\">Expected file: <code>{}</code></div>",
                html_escape(value)
            )
        })
        .unwrap_or_else(|| {
            "<div class=\"info muted\">Host did not specify a file name.</div>".to_string()
        });

    let protocol_url = room
        .as_ref()
        .map(|room_id| build_protocol_url(room_id, code.as_deref(), file.as_deref()));

    let launch_section = protocol_url
        .as_ref()
        .map(|url| {
            format!(
                "<a class=\"primary\" href=\"{href}\">Open Hang Client</a>",
                href = html_escape_attr(url)
            )
        })
        .unwrap_or_else(|| {
            "<p class=\"muted\">Missing room code. Ask the host for a valid invite link.</p>"
                .to_string()
        });

    let auto_launch_script = protocol_url
        .as_ref()
        .map(|url| {
            let js_url =
                serde_json::to_string(url).unwrap_or_else(|_| "\"hang://join\"".to_string());
            format!(
                "<script>setTimeout(function(){{window.location.href={};}}, 450);</script>",
                js_url
            )
        })
        .unwrap_or_default();

    format!(
        r#"<!DOCTYPE html>
<html lang=\"en\">
  <head>
    <meta charset=\"utf-8\" />
    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />
    <title>Hang Invite</title>
    <style>
      :root {{
        color-scheme: dark;
        --bg: #060606;
        --card: rgba(14, 14, 14, 0.9);
        --accent: #ff8a00;
        --text: #f4f4f4;
        --muted: #9f9f9f;
      }}
      body {{
        margin: 0;
        font-family: 'Inter', system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
        min-height: 100vh;
        background: radial-gradient(circle at top, rgba(255, 138, 0, 0.2), transparent 45%), var(--bg);
        color: var(--text);
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 3rem 1.25rem;
      }}
      .card {{
        width: min(520px, 100%);
        background: var(--card);
        border-radius: 28px;
        padding: clamp(1.75rem, 4vw, 3rem);
        box-shadow: 0 20px 70px rgba(0, 0, 0, 0.45);
        border: 1px solid rgba(255, 255, 255, 0.05);
      }}
      h1 {{
        margin-top: 0;
        font-size: 1.8rem;
        letter-spacing: 0.01em;
      }}
      .info {{
        margin-top: 1rem;
        font-size: 1rem;
      }}
      .muted {{
        color: var(--muted);
      }}
      code {{
        background: rgba(255, 255, 255, 0.08);
        padding: 0.25rem 0.45rem;
        border-radius: 0.65rem;
        font-size: 0.95rem;
      }}
      .actions {{
        margin-top: 2rem;
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
      }}
      .primary {{
        background: linear-gradient(135deg, #ff8a00, #ff6c00);
        color: #050505;
        text-decoration: none;
        text-align: center;
        font-weight: 600;
        padding: 0.9rem 1rem;
        border-radius: 999px;
      }}
      .secondary {{
        border: 1px solid rgba(255, 255, 255, 0.15);
        border-radius: 999px;
        text-align: center;
        padding: 0.85rem 1rem;
        color: var(--text);
        text-decoration: none;
        font-weight: 500;
      }}
    </style>
    {auto_launch_script}
  </head>
  <body>
    <div class=\"card\">
      <h1>{heading}</h1>
      {file_block}
      {passcode_block}
      <div class=\"info muted\">1. Ensure the Hang desktop client is installed.</div>
      <div class=\"info muted\">2. Load the same video file locally before joining.</div>
      <div class=\"actions\">
        {launch_section}
        <a class=\"secondary\" href=\"/downloads/hang-client.exe\">Download Hang Client</a>
      </div>
    </div>
  </body>
</html>
"#,
        heading = heading,
        file_block = file_block,
        passcode_block = passcode_block,
        launch_section = launch_section,
        auto_launch_script = auto_launch_script
    )
}

fn build_protocol_url(room: &str, code: Option<&str>, file: Option<&str>) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("room", room);
    if let Some(passcode) = code.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }) {
        serializer.append_pair("code", passcode);
    }
    if let Some(file_name) = file.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }) {
        serializer.append_pair("file", file_name);
    }
    format!("hang://join?{}", serializer.finish())
}

fn html_escape(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn html_escape_attr(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

async fn handle_message(
    text: &str,
    client_id: Uuid,
    state: &ServerState,
    client_senders: &ClientSenders,
) -> anyhow::Result<()> {
    let msg: Message = serde_json::from_str(text)?;

    match msg {
        Message::CreateRoom {
            file_hash,
            passcode,
        } => {
            let canonical_hash = file_hash.clone();
            let (room_id, passcode_enabled) = state.create_room(client_id, file_hash, passcode);
            let resume_token = state.remember_session(client_id, &room_id, &canonical_hash, true);
            if let Some(tx) = client_senders.read().await.get(&client_id) {
                let _ = tx.send(Message::RoomCreated {
                    room_id: room_id.clone(),
                    client_id,
                    passcode_enabled,
                    file_hash: canonical_hash,
                    resume_token,
                });
            }
            broadcast_member_count(&state, client_senders, &room_id).await;
        }

        Message::JoinRoom {
            room_id,
            file_hash,
            passcode,
        } => {
            let response = match state
                .join_room(client_id, &room_id, &file_hash, passcode)
                .await
            {
                Ok((is_host, canonical_hash)) => {
                    let resume_token =
                        state.remember_session(client_id, &room_id, &canonical_hash, is_host);
                    Message::RoomJoined {
                        room_id: room_id.clone(),
                        client_id,
                        is_host,
                        passcode_enabled: state
                            .rooms
                            .get(&room_id)
                            .map(|room| room.passcode_hash.is_some())
                            .unwrap_or(false),
                        file_hash: canonical_hash,
                        resume_token,
                    }
                }
                Err(e) if e.contains("not found") => Message::RoomNotFound,
                Err(e) if e.contains("mismatch") => {
                    let room = state.rooms.get(&room_id);
                    let expected = room.map(|r| r.file_hash.clone()).unwrap_or_default();
                    Message::FileHashMismatch { expected }
                }
                Err(e) => Message::Error { message: e },
            };

            if let Some(tx) = client_senders.read().await.get(&client_id) {
                let _ = tx.send(response);
            }

            broadcast_member_count(&state, client_senders, &room_id).await;
        }

        Message::LeaveRoom => {
            if let Some(room_id) = state.leave_room(client_id).await {
                broadcast_member_count(&state, client_senders, &room_id).await;
            }
            state.clear_session(client_id);
            if let Some(tx) = client_senders.read().await.get(&client_id) {
                let _ = tx.send(Message::RoomLeft);
            }
        }
        Message::ResumeSession { token } => {
            let response = state.resume_session(client_id, &token).await;
            if let Some(tx) = client_senders.read().await.get(&client_id) {
                match response {
                    Ok(outcome) => {
                        let _ = tx.send(Message::RoomJoined {
                            room_id: outcome.room_id.clone(),
                            client_id,
                            is_host: outcome.was_host,
                            passcode_enabled: outcome.passcode_enabled,
                            file_hash: outcome.file_hash.clone(),
                            resume_token: outcome.resume_token.clone(),
                        });
                        broadcast_member_count(&state, client_senders, &outcome.room_id).await;
                    }
                    Err(err) => {
                        let _ = tx.send(Message::Error { message: err });
                    }
                }
            }
        }

        Message::SyncCommand(command) => {
            // Get client's room
            let room_id = state
                .clients
                .get(&client_id)
                .and_then(|c| c.room_id.clone());

            if let Some(room_id) = room_id {
                // Broadcast to all room members
                broadcast_to_room(state, client_senders, &room_id, client_id, command).await;
            }
        }

        _ => {
            tracing::warn!("Unexpected message from client: {:?}", msg);
        }
    }

    Ok(())
}

async fn broadcast_to_room(
    state: &ServerState,
    client_senders: &ClientSenders,
    room_id: &str,
    from_client: Uuid,
    command: SyncCommand,
) {
    let members = state.get_room_members(room_id).await;
    let senders = client_senders.read().await;

    tracing::debug!(
        "Broadcasting {:?} from {} to {} members in room {}",
        command,
        from_client,
        members.len(),
        room_id
    );

    let broadcast_msg = Message::SyncBroadcast {
        from_client,
        command,
    };

    for member_id in members {
        if let Some(tx) = senders.get(&member_id) {
            let _ = tx.send(broadcast_msg.clone());
        }
    }
}

async fn broadcast_member_count(
    state: &ServerState,
    client_senders: &ClientSenders,
    room_id: &str,
) {
    let members = state.get_room_members(room_id).await;
    let count = members.len();
    if count == 0 {
        return;
    }
    let senders = client_senders.read().await;
    for member_id in members {
        if let Some(tx) = senders.get(&member_id) {
            let _ = tx.send(Message::RoomMemberUpdate {
                room_id: room_id.to_string(),
                members: count,
            });
        }
    }
}
