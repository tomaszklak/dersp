use self::data::{ClientInfo, Frame, FrameType, ServerInfo, ServerKey};

use crate::crypto::{PublicKey, SecretKey};
use anyhow::{anyhow, ensure};
use codec::{Decode, Encode};
use log::debug;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub mod data;
const UPGRADE_MSG_SIZE: usize = 4096;

pub async fn handle_handshake<RW: AsyncWrite + AsyncRead + Unpin>(
    mut rw: &mut RW,
    sk: &SecretKey,
) -> anyhow::Result<(PublicKey, Option<String>)> {
    finalize_http_phase(&mut rw).await?;

    write_server_key(&mut rw, &sk).await?;

    let (pk, meshkey) = read_client_info(&mut rw, &sk).await?;

    write_server_info(&mut rw).await?;

    Ok((pk, meshkey))
}

async fn finalize_http_phase<RW: AsyncWrite + AsyncRead + Unpin>(
    rw: &mut RW,
) -> anyhow::Result<()> {
    let mut buf = [0u8; UPGRADE_MSG_SIZE];
    let n = rw.read(&mut buf).await?; // TODO: timeout
    ensure!(n > 0, "empty initiall message");
    ensure!(n < UPGRADE_MSG_SIZE, "initial message too big");

    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut req = httparse::Request::new(&mut headers);
    let body_start = req.parse(&buf)?; // TODO: add context
    ensure!(body_start.is_complete());
    validate_headers(&headers)?;
    let body_start = body_start.unwrap();
    let _body = &buf[body_start..];
    // TODO: do something with body?
    rw.write(b"HTTP/1.1 200 OK\r\n\r\n").await?;

    Ok(())
}

fn validate_headers(headers: &[httparse::Header]) -> anyhow::Result<()> {
    for h in headers {
        if h.name == "Upgrade" {
            let value = std::str::from_utf8(h.value)?.to_ascii_lowercase();
            ensure!(
                value == "websocket" || value == "derp",
                "Unexpected Upgrade value {value}"
            );
        }

        if h.name == "Connection" {
            let value = std::str::from_utf8(h.value)?.to_ascii_lowercase();
            ensure!(value == "upgrade", "Unexpected Connection value {value}");
        }
    }

    Ok(())
}

async fn write_server_key<W: AsyncWrite + Unpin>(
    writer: &mut W,
    secret_key: &SecretKey,
) -> anyhow::Result<()> {
    let mut server_key = ServerKey::new(secret_key.public());
    let mut buf = Vec::new();
    server_key.frame().encode(&mut buf)?;
    writer.write_all(&buf).await.map_err(|e| anyhow!("{}", e))
}

async fn read_client_info<R: AsyncRead + Unpin>(
    reader: &mut R,
    sk: &SecretKey,
) -> anyhow::Result<(PublicKey, Option<String>)> {
    // TODO use only one prealocated buffer for read / write
    let mut buf = [0; 1024];
    reader.read(&mut buf).await?;

    let client_info = match FrameType::get_frame_type(&buf) {
        FrameType::ClientInfo => {
            Frame::<ClientInfo>::decode(&mut buf.as_slice()).map_err(|_| anyhow!("Decode error"))
        }
        ty => anyhow::bail!("Unexpected message: {:?}", ty),
    }?;
    let client_info = client_info.inner.into_inner();
    debug!("Client public key: {:?}", client_info.public_key);

    let complete_info = client_info.complete(sk)?;

    debug!("client info: {:?}", complete_info.payload);

    Ok((
        complete_info.public_key,
        if complete_info.payload.meshkey.is_empty() {
            None
        } else {
            Some(complete_info.payload.meshkey)
        },
    ))
}

async fn write_server_info<W: AsyncWrite + Unpin>(writer: &mut W) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    ServerInfo::default().frame().encode(&mut buf)?;
    writer.write_all(&buf).await.map_err(|e| anyhow!("{e}"))
}
