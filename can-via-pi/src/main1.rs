use actix_files as fs;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use serde::Serialize;
use socketcan::embedded_can::{Frame as EmbeddedFrame, Id}; // For id(), data(), is_extended(), is_remote_frame()
use socketcan::{CanSocket, Frame as SocketcanFrame, Socket}; // For is_error_frame()
use std::error::Error;
use std::sync::{Arc, Mutex};
use tokio::{signal, task};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Serialize, Clone, Debug)]
struct CANMessage {
    id: u32,
    data: Vec<u8>,
    is_extended: bool,
    is_error: bool,
    is_rtr: bool,
}

#[derive(Clone)]
struct AppState {
    messages: Arc<Mutex<Vec<CANMessage>>>,
}

async fn index() -> impl Responder {
    HttpResponse::Ok().body("CAN Bus Web Interface")
}

async fn get_messages(data: web::Data<AppState>) -> impl Responder {
    let messages = data.messages.lock().unwrap();
    log::debug!("Messages in state: {:?}", *messages);

    if messages.is_empty() {
        log::debug!("No messages available");
        return HttpResponse::Ok().json(vec!["No messages available"]);
    }

    HttpResponse::Ok().json(&*messages)
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init(); // Initialise logging
    log::debug!("Starting CAN Bus Web Interface...");

    let state = AppState {
        messages: Arc::new(Mutex::new(Vec::new())),
    };

    let can_state = state.messages.clone();
    let running = Arc::new(AtomicBool::new(true));
    let can_running = running.clone();

    // Spawn a task to read CAN frames
    let can_task = task::spawn_blocking(move || {
        let socket = CanSocket::open("can0").expect("Failed to open can0");
        log::debug!("Socket opened successfully.");

        while can_running.load(Ordering::Relaxed) {
            match socket.read_frame() {
                Ok(can_frame) => {
                    log::debug!("CAN frame received: {:?}", can_frame);

                    let message = CANMessage {
                        id: match can_frame.id() {
                            Id::Standard(id) => id.as_raw().into(),
                            Id::Extended(id) => id.as_raw(),
                        },
                        data: can_frame.data().to_vec(),
                        is_extended: can_frame.is_extended(),
                        is_error: can_frame.is_error_frame(),
                        is_rtr: can_frame.is_remote_frame(),
                    };

                    let mut messages = can_state.lock().unwrap();
                    log::debug!("Storing message: {:?}", message);
                    messages.push(message);
                    // Keep only the last 100 messages
                    if messages.len() > 100 {
                        messages.remove(0);
                    }
                }
                Err(e) => log::error!("Error reading CAN frame: {}", e),
            }
        }
        log::info!("CAN task shutting down...");
    });

    let server = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            // Define specific routes first
            .route("/", web::get().to(index))
            .route("/messages", web::get().to(get_messages))
            // Then serve static files
            .service(fs::Files::new("/", "./static").index_file("index.html"))
    })
    .bind("0.0.0.0:8080")?
    .run();

    // Handle Ctrl-C signal
    tokio::select! {
        _ = signal::ctrl_c() => {
            log::info!("Ctrl-C received, shutting down...");
            running.store(false, Ordering::Relaxed);
        }
        _ = server => log::info!("Server has shut down"),
    }

    can_task.await.unwrap(); // Wait for CAN task to exit

    Ok(())
}
