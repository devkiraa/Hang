use axum::{
    extract::{
        ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade},
        State,
    },
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

mod protocol;
mod state;

use protocol::{Message, SyncCommand};
use state::ServerState;

type ClientSender = mpsc::UnboundedSender<Message>;
type ClientSenders = Arc<RwLock<HashMap<Uuid, ClientSender>>>;

#[derive(Clone)]
struct AppState {
    server_state: ServerState,
    client_senders: ClientSenders,
}

const INDEX_HTML: &str = include_str!("../static/index.html");

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
        .route("/healthz", get(health_check))
        .route("/ws", get(ws_endpoint))
        .with_state(app_state.clone());

    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Hang Server listening on {}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_endpoint(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_connection(socket, state))
}

async fn serve_index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn health_check() -> &'static str {
    "ok"
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

async fn handle_message(
    text: &str,
    client_id: Uuid,
    state: &ServerState,
    client_senders: &ClientSenders,
) -> anyhow::Result<()> {
    let msg: Message = serde_json::from_str(text)?;

    match msg {
        Message::CreateRoom { file_hash } => {
            let room_id = state.create_room(client_id, file_hash);
            if let Some(tx) = client_senders.read().await.get(&client_id) {
                let _ = tx.send(Message::RoomCreated {
                    room_id: room_id.clone(),
                    client_id,
                });
            }
            broadcast_member_count(&state, client_senders, &room_id).await;
        }

        Message::JoinRoom { room_id, file_hash } => {
            let response = match state.join_room(client_id, &room_id, &file_hash).await {
                Ok(is_host) => Message::RoomJoined {
                    room_id: room_id.clone(),
                    client_id,
                    is_host,
                },
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
            if let Some(tx) = client_senders.read().await.get(&client_id) {
                let _ = tx.send(Message::RoomLeft);
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
