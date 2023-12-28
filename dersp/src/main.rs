mod crypto;
mod proto;

use log::{debug, info, warn};
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};

use crate::crypto::SecretKey;
use crate::proto::Server;

async fn handle_client(socket: TcpStream, peer_addr: SocketAddr) -> anyhow::Result<()> {
    debug!("Got connection from: {peer_addr:?}");
    let mut server = Server::new(socket, SecretKey::gen());

    // TODO read / write loop
    // Here should the read / write be done and handle fn should take input data and
    // return output data. Or add a connection handler with callback for output.
    server.handle().await?;

    Ok(())
}

#[tokio::main]
pub async fn main() {
    env_logger::init();
    let listener = TcpListener::bind("127.0.0.1:8800").await.unwrap();
    info!("Listening on: {:?}", listener.local_addr());

    // Accept all incoming TCP connections.
    loop {
        if let Ok((socket, peer_addr)) = listener.accept().await {
            tokio::spawn(async move {
                if let Err(e) = handle_client(socket, peer_addr).await {
                    warn!("Client {peer_addr:?} failed: {e:?}");
                }
            });
        }
    }
}
