use anyhow::Result;
use serde_json;
use tokio::{
    io::AsyncReadExt,
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::mpsc::UnboundedSender,
};
use tracing::warn;

use crate::invite::InviteSignal;

const IPC_ADDR: &str = "127.0.0.1:39275";

pub async fn start_invite_listener(tx: UnboundedSender<InviteSignal>) -> bool {
    match TcpListener::bind(IPC_ADDR).await {
        Ok(listener) => {
            tokio::spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok((mut socket, _)) => {
                            let mut buf = Vec::new();
                            match socket.read_to_end(&mut buf).await {
                                Ok(_) => {
                                    if let Ok(text) = String::from_utf8(buf) {
                                        if let Ok(signal) =
                                            serde_json::from_str::<InviteSignal>(&text)
                                        {
                                            let _ = tx.send(signal);
                                        }
                                    }
                                }
                                Err(e) => warn!("Failed to read invite IPC message: {}", e),
                            }
                        }
                        Err(e) => {
                            warn!("Invite IPC accept error: {}", e);
                            break;
                        }
                    }
                }
            });
            true
        }
        Err(e) => {
            warn!("Invite IPC listener unavailable: {}", e);
            false
        }
    }
}

pub async fn send_invite_to_primary(signal: InviteSignal) -> Result<()> {
    let mut stream = TcpStream::connect(IPC_ADDR).await?;
    let payload = serde_json::to_vec(&signal)?;
    stream.write_all(&payload).await?;
    Ok(())
}
