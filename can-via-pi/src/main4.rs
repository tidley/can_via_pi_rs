use actix_files as fs;
use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_ws::{Message, Session};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

#[derive(Debug, Serialize, Deserialize, Clone)]
struct CANMessage {
    id: u32,
    data: Vec<u8>,
    is_extended: bool,
    is_error: bool,
    is_rtr: bool,
}

struct AppState {
    messages: Arc<Mutex<Vec<CANMessage>>>,
    sender: broadcast::Sender<CANMessage>,
}

/// Return messages from memory as JSON, optionally filtered by `?filter_id=0x277&filter_id=0x123` etc.
async fn messages_handler(
    data: web::Data<AppState>,
    query: web::Query<HashMap<String, String>>,
) -> Result<impl Responder, Error> {
    // Collect all `filter_id` parameters from the query string
    let mut filter_ids = Vec::new();
    for (key, val) in query.iter() {
        if key == "filter_id" {
            // parse hex or decimal, ignoring errors
            if let Ok(id) = u32::from_str_radix(val.trim_start_matches("0x"), 16)
                .or_else(|_| val.parse::<u32>())
            {
                filter_ids.push(id);
            }
        }
    }

    let lock = data.messages.lock().await;
    let messages: Vec<_> = lock
        .iter()
        .filter(|m| filter_ids.is_empty() || filter_ids.contains(&m.id))
        .cloned()
        .collect();

    Ok(web::Json(messages))
}

/// Simple WebSocket handler that streams broadcast messages to the client
async fn ws_handler(
    req: HttpRequest,
    stream: web::Payload,
    data: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    let (res, mut session, mut msg_stream) = actix_ws::handle(&req, stream)?;

    // Subscribe to broadcast messages
    let mut local_rx = data.sender.subscribe();

    // Spawn a task to handle all incoming frames and broadcasts
    actix_web::rt::spawn(async move {
        loop {
            tokio::select! {
                // Inbound messages
                Some(Ok(msg)) = msg_stream.next() => {
                    match msg {
                        Message::Text(text) => {
                            // Echo text
                            if session.text(text).await.is_err() {
                                break;
                            }
                        }
                        Message::Binary(bin) => {
                            // Echo binary
                            if session.binary(bin).await.is_err() {
                                break;
                            }
                        }
                        Message::Ping(bytes) => {
                            // Respond to ping
                            if session.pong(&bytes).await.is_err() {
                                break;
                            }
                        }
                        Message::Close(reason) => {
                            // Handle close
                            let _ = session.close(reason).await;
                            break;
                        }
                        // Ignore other frames
                        _ => {}
                    }
                }
                // Outbound broadcast messages
                Ok(can_msg) = local_rx.recv() => {
                    if let Ok(json) = serde_json::to_string(&can_msg) {
                        if session.text(json).await.is_err() {
                            break;
                        }
                    }
                }
                // If everything ends, exit
                else => { break; }
            }
        }
    });

    // Return the handshake response (WebSocket established)
    Ok(res)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();

    let (sender, _) = broadcast::channel(100);

    let state = web::Data::new(AppState {
        messages: Arc::new(Mutex::new(Vec::new())),
        sender,
    });

    // Example background task that populates CAN messages
    let state_clone = state.clone();
    tokio::spawn(async move {
        loop {
            let can_message = CANMessage {
                id: 0x277,
                data: vec![1, 2, 3, 4],
                is_extended: false,
                is_error: false,
                is_rtr: false,
            };

            let _ = state_clone.sender.send(can_message.clone());
            let mut lock = state_clone.messages.lock().await;
            lock.push(can_message);

            if lock.len() > 100 {
                lock.remove(0);
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .service(fs::Files::new("/", "./static").index_file("index.html"))
            // WebSocket route
            .route("/ws", web::get().to(ws_handler))
            // JSON messages route (fetched by the front end)
            .route("/messages", web::get().to(messages_handler))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}
