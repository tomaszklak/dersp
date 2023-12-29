use self::data::{
    ClientInfo, ForwardPacket, Frame, FrameType, PeerPresent, ServerInfo, ServerKey, WatchConns,
};

use crate::crypto::{PublicKey, SecretKey};
use anyhow::{anyhow, ensure};
use codec::{Decode, Encode, SizeWrapper};
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
    let server_key = ServerKey::new(secret_key.public());
    let mut buf = Vec::new();
    server_key.frame().encode(&mut buf)?;
    writer.write_all(&buf).await.map_err(|e| anyhow!("{}", e))
}

async fn read_server_key<R: AsyncRead + Unpin>(reader: &mut R) -> anyhow::Result<PublicKey> {
    let mut buf = [0; 1024];
    reader.read(&mut buf).await?;

    let server_key = match FrameType::get_frame_type(&buf) {
        FrameType::ServerKey => Frame::<ServerKey>::decode(&mut buf.as_slice())
            .map_err(|_| anyhow!("Decode error"))?
            .inner
            .into_inner(),
        ty => anyhow::bail!("Unexpected message: {ty:?}"),
    };

    server_key.validate_magic()?;

    Ok(server_key.public_key)
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
        ty => anyhow::bail!("Unexpected message: {ty:?}"),
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

async fn write_client_info<W: AsyncWrite + Unpin>(
    writer: &mut W,
    client_info: ClientInfo,
) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    client_info.frame().encode(&mut buf)?;
    writer.write_all(&buf).await.map_err(|e| anyhow!("{e}"))
}

async fn write_server_info<W: AsyncWrite + Unpin>(writer: &mut W) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    ServerInfo::default().frame().encode(&mut buf)?;
    writer.write_all(&buf).await.map_err(|e| anyhow!("{e}"))
}

pub async fn read_server_info<R: AsyncRead + Unpin>(reader: &mut R) -> anyhow::Result<()> {
    let mut buf = [0; 1024];
    reader.read(&mut buf).await?;

    let ty = FrameType::get_frame_type(&buf);
    if ty != FrameType::ServerInfo {
        Err(anyhow::anyhow!("Invalid frame type {ty:?}"))
    } else {
        Ok(())
    }
}

pub async fn write_peer_present<W: AsyncWrite + Unpin>(
    writer: &mut W,
    public_key: &PublicKey,
) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    let peer_present = Frame {
        frame_type: data::FrameType::PeerPresent,
        inner: SizeWrapper::new(PeerPresent {
            public_key: *public_key,
        }),
    };
    peer_present.encode(&mut buf)?;
    writer.write_all(&buf).await.map_err(|e| anyhow!("{e}"))
}

pub async fn write_forward_packet<W: AsyncWrite + Unpin>(
    writer: &mut W,
    forward_packet: ForwardPacket,
) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    forward_packet.frame().encode(&mut buf)?;
    writer.write_all(&buf).await.map_err(|e| anyhow!("{e}"))
}

pub async fn write_watch_conns<W: AsyncWrite + Unpin>(writer: &mut W) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    let frame = Frame {
        frame_type: FrameType::WatchConns,
        inner: SizeWrapper::new(WatchConns::default()),
    };
    frame.encode(&mut buf)?;
    writer.write_all(&buf).await.map_err(|e| anyhow!("{e}"))
}

/// Reads the server key and sends the initiation message via a writer to the DERP server
/// Initiation message consists of:
/// * `public key`
/// * `nonce` - a random byte sequence generated by client
/// * `ciphertext` - an initiation JSON encrypted with the secret key, using a generated nonce
pub async fn exchange_keys<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut reader: R,
    mut writer: W,
    secret_key: SecretKey,
    meshkey: Option<&str>,
) -> anyhow::Result<PublicKey> {
    let server_key = read_server_key(&mut reader).await?;
    debug!("server key: {server_key}");
    let client_info = ClientInfo::new(secret_key, server_key, meshkey)?;
    write_client_info(&mut writer, client_info).await?;
    Ok(server_key)
}
