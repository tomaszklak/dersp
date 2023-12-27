mod crypto;
mod proto;

use anyhow::{anyhow, bail, ensure, Context};

use crypto_box::{
    aead::{Aead, AeadCore},
    SalsaBox,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::crypto::{PublicKey, SecretKey};
use crate::proto::{read_frame, write_frame, FrameType, MAGIC};

const UPGRADE_MSG_SIZE: usize = 4096;

#[derive(Debug, Serialize, Deserialize)]
struct ClientInfoPayload {
    version: u32,
    #[serde(rename = "meshKey")]
    meshkey: String,
}

async fn handle_client(mut socket: TcpStream, peer_addr: SocketAddr) -> anyhow::Result<()> {
    println!("Got connection from: {peer_addr:?}");
    let mut buf = [0u8; UPGRADE_MSG_SIZE];
    let n = socket.read(&mut buf).await?; // TODO: timeout
    ensure!(n > 0, "empty initiall message");
    ensure!(n < UPGRADE_MSG_SIZE, "initial message too big");

    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut req = httparse::Request::new(&mut headers);
    let body_start = req.parse(&buf)?; // TODO: add context
    ensure!(body_start.is_complete());
    validate_headers(&headers)?;
    let body_start = body_start.unwrap();
    let body = &buf[body_start..];
    // TODO: do something with body?

    println!("hello: {headers:?}");
    socket.write(b"HTTP/1.1 200 OK\r\n\r\n").await?;

    let sk = SecretKey::gen();
    let pk = sk.public();
    let mut buf = vec![];
    buf.extend_from_slice(&MAGIC);
    buf.extend_from_slice(&pk);

    write_frame(&mut socket, FrameType::ServerKey, buf)
        .await
        .map_err(|e| anyhow!("{}", e))?;

    let client_info = match dbg!(read_frame(&mut socket).await).map_err(|e| anyhow!("{e}"))? {
        (FrameType::ClientInfo, buf) => {
            let client_pk = buf.get(..32).unwrap();
            let nonce = buf.get(32..(32 + 24)).unwrap();
            let cipher_text = buf.get((32 + 24)..).unwrap();
            let client_pk: PublicKey = client_pk.try_into()?;
            let b = SalsaBox::new(&client_pk.into(), &sk.into());
            let plain_text = b.decrypt(nonce.try_into()?, cipher_text)?;
            println!("{}", std::str::from_utf8(&plain_text).unwrap());

            let client_info: ClientInfoPayload =
                serde_json::from_slice(&plain_text).with_context(|| "Client info parsing")?;
            println!("client info: '{client_info:?}'");
        }
        (frame_type, _) => {
            bail!("Unexpected message: {frame_type:?}");
        }
    };

    write_frame(&mut socket, FrameType::ServerInfo, Vec::new())
        .await
        .map_err(|e| anyhow!("{e}"))?;

    Ok(())
}

fn validate_headers(_headers: &[httparse::Header]) -> anyhow::Result<()> {
    // TODO
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
