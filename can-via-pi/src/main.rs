use socketcan::{CanSocket, Socket}; // Import both `CanSocket` and the `Socket` trait
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    // Open a CAN socket bound to can0
    let socket = CanSocket::open("can0")?;

    println!("Listening on can0... Press Ctrl+C to exit.");

    loop {
        // Read a CAN frame
        match socket.read_frame() {
            Ok(frame) => println!("{:?}", frame),
            Err(e) => {
                eprintln!("Error reading CAN frame: {}", e);
                break; // Exit loop on error
            },
        }
    }

    Ok(())
}
