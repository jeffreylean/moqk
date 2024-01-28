pub mod server;

use core::panic;
use moqk::server::new_with_port;
use std::{thread, time::Duration};

#[tokio::main]
async fn main() {
    // start the server
    let s = new_with_port(9000);
    match s.start().await {
        Ok(_) => println!("Server is running..."),
        Err(e) => panic!("Error starting server: {}", e),
    }

    thread::sleep(Duration::from_secs(10));
}
