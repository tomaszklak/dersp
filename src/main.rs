mod crypto;
mod proto;

use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};

use crate::proto::handle_handshake;
use crate::{
    crypto::{PublicKey, SecretKey},
};

async fn handle_client(mut socket: TcpStream, peer_addr: SocketAddr) -> anyhow::Result<()> {
    println!("Got connection from: {peer_addr:?}");
    let sk = SecretKey::gen();
    handle_handshake(&mut socket, &sk).await?;

    // TODO read / write loop

    Ok(())
}

#[tokio::main]
pub async fn main() {
    let listener = TcpListener::bind("127.0.0.1:8800").await.unwrap();
    println!("listening on: {:?}", listener.local_addr());

    // Accept all incoming TCP connections.
    loop {
        if let Ok((socket, peer_addr)) = listener.accept().await {
            // Spawn a new task to process each connection.
            tokio::spawn(async move {
                if let Err(e) = handle_client(socket, peer_addr).await {
                    println!("Client {peer_addr:?} failed: {e:?}");
                }
            });
        }
    }
}
