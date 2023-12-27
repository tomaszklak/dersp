mod crypto;
mod proto;

use anyhow::{anyhow, bail, ensure, Context};

use crypto_box::{aead::Aead, SalsaBox};
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};

use crate::proto::{
    finalize_http_phase, read_frame, write_server_info, write_server_key, FrameType,
};
use crate::{
    crypto::{PublicKey, SecretKey},
    proto::ClientInfoPayload,
};

async fn handle_client(mut socket: TcpStream, peer_addr: SocketAddr) -> anyhow::Result<()> {
    println!("Got connection from: {peer_addr:?}");
    finalize_http_phase(&mut socket).await?;

    let sk = SecretKey::gen();
    write_server_key(&mut socket, &sk).await?;
    // server key end

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
            ensure!(
                client_info
                    == ClientInfoPayload {
                        version: 2,
                        meshkey: "".to_owned()
                    }
            )
        }
        (frame_type, _) => {
            bail!("Unexpected message: {frame_type:?}");
        }
    };

    // client info end

    write_server_info(&mut socket).await?;

    // server info end

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
