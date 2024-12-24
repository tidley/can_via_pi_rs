use actix_files as fs;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use serde::{Deserialize, Serialize};
use socketcan::embedded_can::{ExtendedId, Frame as EmbeddedFrame, Id, StandardId};
use socketcan::{CanFrame, CanSocket, Frame as SocketcanFrame, Socket}; // Import Frame to use is_error_frame()
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::{signal, task};

#[derive(Serialize, Clone, Debug)]
struct CANMessage {
    id: u32,
    data: Vec<u8>,
    is_extended: bool,
    is_error: bool,
    is_rtr: bool,
}

#[derive(Deserialize)]
struct FilterRequest {
    filter_id: Option<u32>,
}

#[derive(Deserialize)]
struct SendRequest {
    id: u32,
    data: Vec<u8>,
    is_extended: bool,
    is_rtr: bool,
}

#[derive(Clone)]
struct AppState {
    messages: Arc<Mutex<Vec<CANMessage>>>,
    can_socket: Arc<Mutex<CanSocket>>,
}

// async fn index() -> impl Responder {
//     HttpResponse::Ok().body("CAN Bus Web Interface")
// }

async fn get_messages(data: web::Data<AppState>, query: web::Query<FilterRequest>) -> impl Responder {
    let filter_id = query.filter_id.unwrap_or(0); // Default to 0 if not provided
    let messages = data.messages.lock().unwrap();

    if messages.is_empty() {
        return HttpResponse::Ok().json(vec!["No messages available"]);
    }

    let filtered_messages: Vec<_> = messages
        .iter()
        .filter(|msg| filter_id == 0 || msg.id == filter_id)
        .cloned()
        .collect();

    HttpResponse::Ok().json(filtered_messages)
}

async fn send_message(
    data: web::Data<AppState>,
    payload: web::Json<SendRequest>,
) -> impl Responder {
    let send_request = payload.into_inner();
    let socket = data.can_socket.lock().unwrap();

    // Construct the CAN frame based on message parameters
    let frame = if send_request.is_extended {
        let id = ExtendedId::new(send_request.id).expect("Invalid extended ID");
        let eid = Id::Extended(id);
        if send_request.is_rtr {
            // Extended RTR frame
            CanFrame::new_remote(eid, send_request.data.len())
                .expect("Failed to create extended RTR frame")
        } else {
            // Extended data frame
            CanFrame::new(eid, &send_request.data).expect("Failed to create extended data frame")
        }
    } else {
        let sid = StandardId::new(send_request.id as u16).expect("Invalid standard ID");
        let std_id = Id::Standard(sid);
        if send_request.is_rtr {
            // Standard RTR frame
            CanFrame::new_remote(std_id, send_request.data.len())
                .expect("Failed to create standard RTR frame")
        } else {
            // Standard data frame
            CanFrame::new(std_id, &send_request.data).expect("Failed to create standard data frame")
        }
    };

    match socket.write_frame(&frame) {
        Ok(_) => HttpResponse::Ok().body("Message sent"),
        Err(e) => {
            eprintln!("Failed to send CAN frame: {}", e);
            HttpResponse::InternalServerError().body("Failed to send message")
        }
    }
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let can_socket = CanSocket::open("can0").expect("Failed to open can0");

    let state = AppState {
        messages: Arc::new(Mutex::new(Vec::new())),
        can_socket: Arc::new(Mutex::new(can_socket)),
    };

    let can_state = state.messages.clone();
    let running = Arc::new(AtomicBool::new(true));
    let can_running = running.clone();

    // Spawn a task to continuously read CAN frames
    let can_task = task::spawn_blocking(move || {
        let socket = CanSocket::open("can0").expect("Failed to open can0");
        while can_running.load(Ordering::Relaxed) {
            match socket.read_frame() {
                Ok(can_frame) => {
                    let message = CANMessage {
                        id: match can_frame.id() {
                            Id::Standard(id) => id.as_raw().into(),
                            Id::Extended(id) => id.as_raw(),
                        },
                        data: can_frame.data().to_vec(),
                        is_extended: can_frame.is_extended(),
                        is_error: can_frame.is_error_frame(), // Now in scope due to `use socketcan::Frame;`
                        is_rtr: can_frame.is_remote_frame(),
                    };

                    let mut messages = can_state.lock().unwrap();
                    messages.push(message);
                    if messages.len() > 100 {
                        messages.remove(0);
                    }
                }
                Err(e) => eprintln!("Error reading CAN frame: {}", e),
            }
        }
    });

    let server = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            //.route("/", web::get().to(index)) // Remove this line
            .route("/messages", web::get().to(get_messages))
            .route("/send", web::post().to(send_message))
            .route("/favicon.ico", web::get().to(|| async { HttpResponse::Ok().finish() }))
            .service(fs::Files::new("/", "./static").index_file("index.html"))
    })
    .bind("0.0.0.0:8080")?
    .run();

    tokio::select! {
        _ = signal::ctrl_c() => {
            running.store(false, Ordering::Relaxed);
        },
        _ = server => {
            // Code to run after server completes, if any
        }
    }

    can_task.await.unwrap();

    Ok(())
}
