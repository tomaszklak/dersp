mod client;
mod crypto;
mod proto;
mod service;

use crate::service::{DerpService, Service};
use std::sync::Arc;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use crate::proto::handle_handshake;

use log::{debug, info, warn};

use crate::crypto::SecretKey;

async fn handle_client(mut socket: TcpStream, peer_addr: SocketAddr) -> anyhow::Result<()> {
    debug!("Got connection from: {peer_addr:?}");
    let sk = SecretKey::gen();
    handle_handshake(&mut socket, &sk).await?;

    // TODO read / write loop

    Ok(())
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let service: Arc<Mutex<DerpService>> = DerpService::new();
    let listener = TcpListener::bind("127.0.0.1:8800").await.unwrap();
    info!("Listening on: {:?}", listener.local_addr());

    service.run(listener).await
}
