mod crypto;
mod proto;

use log::{debug, info, warn};
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};

use crate::crypto::SecretKey;
use crate::proto::handle_handshake;

async fn handle_client(mut socket: TcpStream, peer_addr: SocketAddr) -> anyhow::Result<()> {
    debug!("Got connection from: {peer_addr:?}");
    let sk = SecretKey::gen();
    handle_handshake(&mut socket, &sk).await?;

    // TODO read / write loop

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
