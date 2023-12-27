mod client;
mod crypto;
mod proto;
mod service;

use log::{debug, info, warn};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::crypto::{PublicKey, SecretKey};
use crate::proto::handle_handshake;
use crate::service::DerpService;

async fn handle_client(
    mut socket: TcpStream,
    peer_addr: SocketAddr,
    service: Arc<Mutex<DerpService>>,
) -> anyhow::Result<()> {
    debug!("Got connection from: {peer_addr:?}");
    let sk = SecretKey::gen();
    let client_pk = handle_handshake(&mut socket, &sk).await?;

    service
        .lock()
        .await
        .start_new_client(socket, client_pk)
        .await?;

    Ok(())
}

#[tokio::main]
pub async fn main() {
    env_logger::init();
    let service: Arc<Mutex<DerpService>> = DerpService::new();
    let listener = TcpListener::bind("127.0.0.1:8800").await.unwrap();
    info!("Listening on: {:?}", listener.local_addr());

    // Accept all incoming TCP connections.
    loop {
        if let Ok((socket, peer_addr)) = listener.accept().await {
            let service = service.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_client(socket, peer_addr, service).await {
                    warn!("Client {peer_addr:?} failed: {e:?}");
                }
            });
        }
    }
}
