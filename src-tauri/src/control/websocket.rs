use std::sync::Arc;
use futures_util::{SinkExt, StreamExt};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

pub struct WebSocketServerState {
    pub broadcast_tx: broadcast::Sender<String>,
}

impl WebSocketServerState {
    pub fn new() -> Self {
        let (broadcast_tx, _) = broadcast::channel(128);
        Self { broadcast_tx }
    }

    pub fn broadcast(&self, message: String) {
        let _ = self.broadcast_tx.send(message);
    }
}

pub fn start_websocket_server<R: Runtime>(
    app: AppHandle<R>,
    port: u16,
    state: Arc<WebSocketServerState>,
) {
    tauri::async_runtime::spawn(async move {
        let addr = format!("0.0.0.0:{}", port);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => {
                println!("[WebSocket Server] Listening on ws://{}", addr);
                l
            }
            Err(e) => {
                eprintln!("[WebSocket Server] Failed to bind to {}: {}", addr, e);
                return;
            }
        };

        while let Ok((stream, peer_addr)) = listener.accept().await {
            let app_handle = app.clone();
            let mut broadcast_rx = state.broadcast_tx.subscribe();

            tauri::async_runtime::spawn(async move {
                let ws_stream = match tokio_tungstenite::accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(e) => {
                        eprintln!("[WebSocket Server] Error during handshake from {}: {}", peer_addr, e);
                        return;
                    }
                };

                let (mut ws_sender, mut ws_receiver) = ws_stream.split();

                // Forward incoming broadcast messages to the connected client
                let mut send_task = tauri::async_runtime::spawn(async move {
                    while let Ok(msg) = broadcast_rx.recv().await {
                        if ws_sender.send(Message::Text(msg)).await.is_err() {
                            break;
                        }
                    }
                });

                // Receive incoming commands from the connected client (Stream Deck / Remote)
                let app_handle_clone = app_handle.clone();
                let mut recv_task = tauri::async_runtime::spawn(async move {
                    while let Some(Ok(message)) = ws_receiver.next().await {
                        if let Message::Text(text) = message {
                            let text_trim = text.trim();
                            // If it's a simple string command like "play_pause", wrap in JSON
                            let payload = if text_trim.starts_with('{') {
                                text_trim.to_string()
                            } else {
                                format!(r#"{{"action":"{}"}}"#, text_trim)
                            };

                            let _ = app_handle_clone.emit("remote-control-action", payload);
                        } else if let Message::Close(_) = message {
                            break;
                        }
                    }
                });

                tokio::select! {
                    _ = (&mut send_task) => recv_task.abort(),
                    _ = (&mut recv_task) => send_task.abort(),
                };
            });
        }
    });
}
